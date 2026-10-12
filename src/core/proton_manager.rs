use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    future::Future,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
    process::{Child, Command, ExitStatus},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    core::{
        fs::{MultiPathOptions, Sizes, extract_file, full_path},
        operations::{Operation, OperationKind, Operations, Phase, operations},
        sys::get_available_storage,
    },
    util::web::{download_file_limited, fetch_data},
};

const COMPONENTS_URL_BASE: &str = "https://aedes.elysiae.app/getComponentInfo";
#[cfg(target_arch = "x86_64")]
const ARCH: &str = "amd64";
#[cfg(target_arch = "aarch64")]
const ARCH: &str = "aarch64";
const MAX_RETRIES: usize = 5;
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const STORAGE_RESERVE: u64 = 64 * 1024 * 1024;
const MARKER: &str = ".elysiae-runtime.json";
const JOURNAL: &str = ".phlogiston-transaction.json";
const TRACKER_SNAPSHOT: &str = "prior-proton-version.json";
const MAX_METADATA_BYTES: u64 = 65536;
const SESSION_FENCE: &str = ".phlogiston-session.json";
const MAX_TAR_ENTRIES: usize = 100_000;
const MAX_TAR_DEPTH: usize = 64;
const MAX_TAR_PATH_BYTES: usize = 4096;
const MAX_TAR_EXTENSION_BYTES: u64 = 64 * 1024;
const MAX_TAR_METADATA_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TAR_DECODED_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const NODE_ALLOCATION: u64 = 8192;

/// Latest Phlogiston release metadata returned by the component service.
#[derive(Debug, Deserialize)]
struct ComponentRelease {
    tag: String,
    prerelease: bool,
    download: ComponentDownload,
}

/// Archive location and expected SHA-256 checksum for a component release.
#[derive(Debug, Deserialize)]
struct ComponentDownload {
    url: String,
    checksum: String,
}

impl ComponentRelease {
    /// Validates release metadata and normalizes its checksum.
    fn validate(&mut self) -> Result<()> {
        ensure!(!self.prerelease, "Refusing prerelease Phlogiston runtime");
        ensure!(valid_version(&self.tag), "Invalid component version");
        self.download.checksum = self.download.checksum.trim().to_ascii_lowercase();
        ensure!(
            valid_checksum(&self.download.checksum),
            "Invalid component checksum"
        );
        let url = reqwest::Url::parse(&self.download.url).context("Invalid component URL")?;
        ensure!(
            url.scheme() == "https" && url.host_str().is_some(),
            "Component URL must use HTTPS"
        );
        Ok(())
    }
}

/// Checks that a version is nonempty, bounded and free of control characters.
fn valid_version(version: &str) -> bool {
    !version.trim().is_empty() && version.len() <= 256 && !version.chars().any(char::is_control)
}

/// Checks for a 64-digit hexadecimal SHA-256 checksum.
fn valid_checksum(checksum: &str) -> bool {
    checksum.len() == 64 && checksum.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Builds the latest Phlogiston release URL for this architecture.
fn component_url() -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(COMPONENTS_URL_BASE)?;
    // latestOnly returns an array; latest returns the single flat release.
    url.query_pairs_mut()
        .append_pair("arch", ARCH)
        .append_pair("component", "phlogiston")
        .append_pair("latest", "true");
    Ok(url)
}

/// Fetches and validates the latest stable Phlogiston release.
async fn latest_release() -> Result<ComponentRelease> {
    let mut release: ComponentRelease = fetch_data(component_url()?.as_str()).await?;
    release.validate()?;
    Ok(release)
}

/// Version stored in the legacy component tracker.
#[derive(Serialize, Deserialize)]
struct LegacyVersion {
    version: String,
}

/// In-tree runtime identity and the transaction that installed it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeVersion {
    format: u32,
    version: String,
    checksum: String,
    transaction: String,
}

/// Reads and validates a runtime's in-tree version marker.
fn runtime_version(root: &Path) -> Result<RuntimeVersion> {
    let marker: RuntimeVersion = read_json(&root.join(MARKER))?;
    ensure!(
        marker.format == 1 && valid_version(&marker.version) && valid_checksum(&marker.checksum),
        "Invalid runtime version marker"
    );
    transaction_id(&marker.transaction)?;
    Ok(marker)
}

// Explicit roots keep tests independent of both the user's profile and each other.
/// Runtime, prefix and transaction paths under an application data root.
struct RuntimePaths(PathBuf);

impl RuntimePaths {
    /// Uses the application's data directory as the runtime root.
    fn appdata() -> Result<Self> {
        Ok(Self(full_path(None, None)?))
    }

    /// Returns the canonical Phlogiston installation path.
    fn live(&self) -> PathBuf {
        self.0.join("phlogiston")
    }

    /// Returns the shared Proton data directory.
    fn prefix(&self) -> PathBuf {
        self.0.join("proton-data")
    }

    /// Returns the runtime transaction journal path.
    fn journal(&self) -> PathBuf {
        self.0.join(JOURNAL)
    }

    /// Returns the durable Wine session fence path.
    fn session(&self) -> PathBuf {
        self.0.join(SESSION_FENCE)
    }

    /// Returns the staging directory owned by this transaction.
    fn stage(&self, journal: &Journal) -> PathBuf {
        self.0.join(format!(".phlogiston-stage-{}", journal.id))
    }

    /// Returns the previous-runtime backup path for this transaction.
    fn backup(&self, journal: &Journal) -> PathBuf {
        self.0.join(format!(".phlogiston-backup-{}", journal.id))
    }

    /// Finds a complete runtime, preferring the canonical layout over legacy ones.
    fn resolve(&self) -> Result<RuntimeLayout> {
        // Legacy layouts are read in place, never silently moved into phlogiston.
        for root in [self.live(), self.0.join("proton"), self.0.clone()] {
            if let Ok(layout) = RuntimeLayout::validate(&root) {
                return Ok(layout);
            }
        }
        bail!(
            "No complete Proton runtime found in {} (install or repair Phlogiston)",
            self.0.display()
        )
    }

    /// Checks whether the installed runtime needs repair or differs from a release.
    fn needs_update(&self, release: &ComponentRelease) -> Result<bool> {
        // A legacy tracker must never bless a missing/corrupt canonical tree.
        if node_exists(&self.live())? {
            return Ok(RuntimeLayout::validate(&self.live()).is_err()
                || runtime_version(&self.live()).map_or(true, |marker| {
                    marker.version != release.tag || marker.checksum != release.download.checksum
                }));
        }
        let Ok(legacy) = self.resolve() else {
            return Ok(true);
        };
        if let Ok(marker) = runtime_version(&legacy.root) {
            return Ok(
                marker.version != release.tag || marker.checksum != release.download.checksum
            );
        }
        let tracker = read_json::<LegacyVersion>(&self.0.join("components/proton.json"));
        Ok(tracker.map_or(true, |marker| marker.version != release.tag))
    }

    /// Updates the legacy tracker from an authoritative runtime marker.
    fn sync_legacy_version(&self, marker: &RuntimeVersion) -> Result<()> {
        real_directory(&self.0.join("components"), true)?;
        atomic_json(
            &self.0.join("components/proton.json"),
            &LegacyVersion {
                version: marker.version.clone(),
            },
        )
    }
}

/// Validated distribution paths for Proton and its matching Wine server.
struct RuntimeLayout {
    root: PathBuf,
    executable: PathBuf,
    wineserver: PathBuf,
}

impl RuntimeLayout {
    /// Validates a runtime using the selected Wine architecture.
    fn validate(root: &Path) -> Result<Self> {
        let (bin, architecture) = wine_architecture(root);
        Self::validate_architecture(root, bin, architecture)
    }

    /// Checks the required runtime files for a specific Wine architecture.
    fn validate_architecture(root: &Path, bin: &str, architecture: &str) -> Result<Self> {
        real_directory(root, false)?;
        let root = root.canonicalize()?;
        let executable = checked_file(&root, "proton", true)?;
        // These are imported/read by Phlogiston before it can run Wine.
        for file in [
            "version",
            "filelock.py",
            "utilities.py",
            "protonfixes/__init__.py",
            "files/share/wine/wine.inf",
        ] {
            checked_file(&root, file, false)?;
        }
        checked_file(&root, &format!("{bin}/wine"), true)?;
        let wineserver = checked_file(&root, &format!("{bin}/wineserver"), true)?;
        // Both sides of ntdll are needed; empty lib directories or a lone fake
        // launcher are not evidence of a usable distribution. Older GE builds
        // kept the 64-bit libraries in lib64 rather than lib.
        ensure!(
            ["lib", "lib64"].into_iter().any(|lib| {
                checked_file(
                    &root,
                    &format!("files/{lib}/wine/{architecture}-unix/ntdll.so"),
                    false,
                )
                .is_ok()
                    && checked_file(
                        &root,
                        &format!("files/{lib}/wine/{architecture}-windows/ntdll.dll"),
                        false,
                    )
                    .is_ok()
            }),
            "Runtime is missing the Wine ntdll layout"
        );
        Ok(Self {
            root,
            executable,
            wineserver,
        })
    }
}

/// Selects native ARM Wine when enabled and available, otherwise x86-64 Wine.
fn wine_architecture(_root: &Path) -> (&'static str, &'static str) {
    #[cfg(target_arch = "aarch64")]
    if std::env::var("PROTON_USE_ARM64").unwrap_or_else(|_| "1".into()) == "1"
        && _root.join("files/bin-arm64").is_dir()
    {
        return ("files/bin-arm64", "aarch64");
    }
    ("files/bin", "x86_64")
}

/// Validates a required runtime file without allowing links outside the distribution.
fn checked_file(root: &Path, relative: &str, executable: bool) -> Result<PathBuf> {
    let path = root.join(relative);
    let resolved = path
        .canonicalize()
        .with_context(|| format!("Missing runtime file {relative}"))?;
    ensure!(
        resolved.starts_with(root),
        "Runtime file escapes its distribution: {relative}"
    );
    let metadata = fs::metadata(&resolved)?;
    ensure!(
        metadata.is_file() && metadata.len() > 0,
        "Invalid runtime file {relative}"
    );
    ensure!(
        !executable || metadata.permissions().mode() & 0o111 != 0,
        "Runtime file is not executable: {relative}"
    );
    // Keep the original path: Proton derives its distribution root from argv[0].
    Ok(path)
}

/// Resolves canonical and supported legacy layouts without writing or executing them.
pub fn runtime_executable() -> Result<PathBuf> {
    Ok(RuntimePaths::appdata()?.resolve()?.executable)
}

/// Checks whether a complete supported Proton runtime is installed.
pub fn components_installed() -> Result<bool> {
    Ok(RuntimePaths::appdata()?.resolve().is_ok())
}

/// Read-only, including when offline or an operation is already active. Scheduling
/// and deciding whether to offer an update belong to the caller.
pub async fn check_runtime_update() -> Result<bool> {
    let release = latest_release().await?;
    RuntimePaths::appdata()?.needs_update(&release)
}

/// The blocking worker, not its cancellable async waiter, owns the lease and all
/// nested download/extraction futures until their filesystem work has quiesced.
async fn runtime_worker(
    operation: Operation,
    work: impl FnOnce(&Operation) -> Result<()> + Send + 'static,
) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let result = work(&operation);
        operation.finish(result)
    })
    .await
    .context("Runtime worker failed")?
}

/// Recovers interrupted work and updates Phlogiston under an operation lease.
pub async fn update_all_modules() -> Result<()> {
    recover_session().await?;
    let operation = operations().begin(OperationKind::Components, None)?;
    let handle = tokio::runtime::Handle::current();
    runtime_worker(operation, move |operation| {
        let paths = RuntimePaths::appdata()?;
        if node_exists(&paths.journal())? {
            operation.set_phase(Phase::Committing)?;
            recover_at(&paths)?;
            operation.set_phase(Phase::Running)?;
        }
        handle.block_on(update_runtime(&paths, operation))
    })
    .await
}

/// Call at startup or before retrying a failed update, never from a read-only
/// installation probe. Recovery also owns the shared-prefix operation lease.
pub async fn recover_runtime() -> Result<()> {
    recover_session().await?;
    let operation = operations().begin(OperationKind::Components, None)?;
    runtime_worker(operation, |operation| {
        operation.set_phase(Phase::Committing)?;
        recover_at(&RuntimePaths::appdata()?)
    })
    .await
}

/// Runs a future until it completes or the operation is cancelled.
async fn cancellable<T>(
    operation: &Operation,
    future: impl Future<Output = Result<T>>,
) -> Result<T> {
    operation.check_cancelled()?;
    tokio::select! {
        result = future => result,
        _ = operation.handle().cancelled_future() => bail!("Operation cancelled"),
    }
}

/// Downloads and installs a needed runtime update, cleaning failed staging.
async fn update_runtime(paths: &RuntimePaths, operation: &Operation) -> Result<()> {
    let release = cancellable(operation, latest_release()).await?;
    operation.check_cancelled()?;
    if !paths.needs_update(&release)? {
        // Reconcile stale compatibility metadata, but do not create a marker in
        // a legacy runtime whose version we cannot independently establish.
        if let Ok(marker) = runtime_version(&paths.live()) {
            operation.set_phase(Phase::Committing)?;
            paths.sync_legacy_version(&marker)?;
        }
        return Ok(());
    }
    operation.check_cancelled()?;
    let journal = start_transaction(paths)?;
    let result = async {
        let stage = paths.stage(&journal);
        let archive = stage.join("download.tar.gz");
        // web/fs accept only paths relative to appdata, not absolute paths.
        let relative_stage = stage.strip_prefix(full_path(None, None)?)?;
        retry_download(operation, || async {
            // A previous checksum failure can leave an archive beside the new
            // transfer's temporary file. Rebudget rather than deleting the old
            // destination when the next HTTP attempt fails.
            let available = available_bytes(&stage)?;
            ensure!(
                available > STORAGE_RESERVE,
                "Insufficient free space for a runtime download"
            );
            let budget = (available - STORAGE_RESERVE).min(MAX_ARCHIVE_BYTES);
            // Do not drop an in-progress file transfer on cancellation: Tokio
            // file writes can outlive their future. Observe cancellation after
            // the transfer returns, while this worker still owns the lease.
            download_file_limited(
                release.download.url.clone(),
                relative_stage.join("download.tar.gz"),
                None,
                None,
                budget,
            )
            .await?;
            operation.check_cancelled()?;
            verify_archive(&archive, &release.download.checksum)
        })
        .await?;
        install_verified_archive(paths, &journal, &release, operation).await
    }
    .await;
    if result.is_err() {
        // Staging errors leave the old runtime untouched. Commit errors perform
        // rollback themselves; an unresolved journal is retained for recovery.
        if read_json::<Journal>(&paths.journal())
            .is_ok_and(|current| current.phase == TransactionPhase::Staging)
            && let Err(error) = discard_transaction(paths, &journal)
        {
            log::warn!("Could not clean runtime staging: {error:#}");
        }
    }
    result
}

/// Extracts, validates and durably commits a checksum-verified runtime archive.
async fn install_verified_archive(
    paths: &RuntimePaths,
    journal: &Journal,
    release: &ComponentRelease,
    operation: &Operation,
) -> Result<()> {
    let stage = paths.stage(journal);
    let relative_stage = stage.strip_prefix(full_path(None, None)?)?;
    operation.set_phase(Phase::Verifying)?;
    check_archive_space(&stage.join("download.tar.gz"), available_bytes(&stage)?)?;
    extract_file(
        MultiPathOptions {
            init_path: relative_stage.join("download.tar.gz"),
            init_path_base_dir: None,
            dest_path: relative_stage.join("runtime"),
            dest_path_base_dir: None,
            overwrite: Some(false),
        },
        Some(true),
    )
    .await?;
    RuntimeLayout::validate(&stage.join("runtime"))?;
    let marker = RuntimeVersion {
        format: 1,
        version: release.tag.clone(),
        checksum: release.download.checksum.clone(),
        transaction: journal.id.clone(),
    };
    atomic_json(&stage.join("runtime").join(MARKER), &marker)?;
    sync_tree(&stage.join("runtime"))?;
    sync_directory(&stage)?;
    operation.set_phase(Phase::Committing)?;
    commit_runtime(paths, journal)
}

/// Retries failed downloads with bounded attempts and cancellable backoff.
async fn retry_download<F, Fut>(operation: &Operation, mut attempt: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    for number in 1..=MAX_RETRIES {
        operation.check_cancelled()?;
        match attempt().await {
            Ok(()) => return Ok(()),
            Err(error) => {
                operation.check_cancelled()?;
                if number == MAX_RETRIES {
                    return Err(error).context(format!(
                        "Runtime download failed after {MAX_RETRIES} attempts"
                    ));
                }
                log::warn!("Runtime download attempt {number}: {error:#}");
                cancellable(operation, async {
                    tokio::time::sleep(Duration::from_millis(250 * number as u64)).await;
                    Ok(())
                })
                .await?;
            }
        }
    }
    unreachable!("MAX_RETRIES is positive")
}

/// Verifies an archive against its expected SHA-256 checksum.
fn verify_archive(archive: &Path, checksum: &str) -> Result<()> {
    ensure!(valid_checksum(checksum), "Invalid component checksum");
    let actual = sha256::try_digest(archive).context("Could not hash runtime archive")?;
    ensure!(
        actual.eq_ignore_ascii_case(checksum),
        "Downloaded component checksum did not match"
    );
    Ok(())
}

/// Returns the free storage available at the runtime destination.
fn available_bytes(destination: &Path) -> Result<u64> {
    Ok(get_available_storage(
        destination.to_str().context("Runtime path is not UTF-8")?,
        Sizes::Bytes,
    )? as u64)
}

/// Charges an archive resource budget, rejecting overflow or excess usage.
fn charge(total: &mut u64, bytes: u64, limit: u64, resource: &str) -> Result<()> {
    *total = total
        .checked_add(bytes)
        .context("Runtime archive resource overflow")?;
    ensure!(
        *total <= limit,
        "Runtime archive exceeds {resource} budget ({total} > {limit})"
    );
    Ok(())
}

/// Bounds raw TAR resources and rejects unsafe extensions before interpretation.
fn raw_tar_preflight(path: &Path) -> Result<()> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(File::open(path)?));
    let mut decoded = 0;
    let mut metadata = 0;
    let mut pax_size = None;
    // tar 0.4 entries() eagerly reads GNU/PAX payloads and GNU sparse maps.
    // raw(true) exposes their headers BEFORE those allocations take place.
    for (index, entry) in archive.entries()?.raw(true).enumerate() {
        ensure!(index < MAX_TAR_ENTRIES, "Too many runtime archive entries");
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        let extension = kind.is_gnu_longname()
            || kind.is_gnu_longlink()
            || kind.is_pax_local_extensions()
            || kind.is_pax_global_extensions();
        ensure!(
            extension
                || kind.is_file()
                || kind.is_dir()
                || kind.is_symlink()
                || kind.is_hard_link(),
            "Unsupported runtime archive entry (including sparse files)"
        );
        let padded = entry
            .size()
            .checked_add(511)
            .context("Runtime archive size overflow")?
            / 512
            * 512;
        charge(&mut decoded, 512, MAX_TAR_DECODED_BYTES, "decoded data")?;
        charge(&mut decoded, padded, MAX_TAR_DECODED_BYTES, "decoded data")?;
        charge(&mut metadata, 512, MAX_TAR_METADATA_BYTES, "metadata")?;
        if extension {
            ensure!(
                entry.size() <= MAX_TAR_EXTENSION_BYTES,
                "Runtime TAR extension is too large"
            );
            charge(
                &mut metadata,
                entry.size(),
                MAX_TAR_METADATA_BYTES,
                "metadata",
            )?;
            if kind.is_pax_local_extensions() || kind.is_pax_global_extensions() {
                // This allocation is now bounded. Raw and normal passes must
                // see the same offsets: PAX size overrides must agree with the
                // next file header, or they could hide unchecked extensions.
                let mut size = None;
                for field in entry.pax_extensions()?.context("Missing PAX metadata")? {
                    let field = field?;
                    ensure!(
                        !field.key_bytes().starts_with(b"GNU.sparse."),
                        "Sparse runtime archives are unsupported"
                    );
                    if field.key_bytes() == b"size" && size.is_none() {
                        size = Some(field.value()?.parse::<u64>().context("Invalid PAX size")?);
                    }
                }
                pax_size = if kind.is_pax_local_extensions() {
                    size
                } else {
                    None
                };
            }
        } else if let Some(size) = pax_size.take() {
            ensure!(
                size == entry.size(),
                "PAX size disagrees with runtime TAR header"
            );
        }
    }
    Ok(())
}

/// Normalizes a relative archive path while enforcing length and depth limits.
fn bounded_archive_path(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.as_os_str().len() <= MAX_TAR_PATH_BYTES,
        "Runtime archive path is too long"
    );
    let mut normalized = PathBuf::new();
    let mut depth = 0;
    for component in path.components() {
        match component {
            Component::Normal(name) => {
                depth += 1;
                ensure!(depth <= MAX_TAR_DEPTH, "Runtime archive path is too deep");
                normalized.push(name);
            }
            Component::CurDir => {}
            _ => bail!("Unsafe runtime archive path"),
        }
    }
    Ok(normalized)
}

/// Checks archive resource limits and budgets staging space, including directories.
fn check_archive_space(path: &Path, available: u64) -> Result<()> {
    // This caller-owned budget also protects the later generic extract_file.
    raw_tar_preflight(path)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(File::open(path)?));
    let mut needed = STORAGE_RESERVE;
    ensure!(needed <= available, "Insufficient staging space");
    let mut metadata = 0;
    let mut directories = HashSet::new();
    for (index, entry) in archive.entries()?.enumerate() {
        ensure!(index < MAX_TAR_ENTRIES, "Too many runtime archive entries");
        let entry = entry?;
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() {
            continue;
        }
        charge(
            &mut metadata,
            entry.path_bytes().len() as u64,
            MAX_TAR_METADATA_BYTES,
            "path metadata",
        )?;
        let path = bounded_archive_path(&entry.path()?)?;
        if let Some(target) = entry.link_name()? {
            ensure!(
                target.as_os_str().len() <= MAX_TAR_PATH_BYTES
                    && target.components().count() <= MAX_TAR_DEPTH,
                "Runtime archive link is too large"
            );
            charge(
                &mut metadata,
                target.as_os_str().len() as u64,
                MAX_TAR_METADATA_BYTES,
                "path metadata",
            )?;
        }
        // Every unique implicit parent consumes an inode/block even when the
        // archive omitted directory entries. Normalize ./ aliases before charging.
        let parents = if kind.is_dir() {
            Some(path.as_path())
        } else {
            path.parent()
        };
        if let Some(parent) = parents {
            for directory in parent.ancestors().filter(|p| !p.as_os_str().is_empty()) {
                if !directories.contains(directory) {
                    ensure!(
                        directories.len() < MAX_TAR_ENTRIES,
                        "Too many runtime archive directories"
                    );
                    charge(
                        &mut metadata,
                        directory.as_os_str().len() as u64,
                        MAX_TAR_METADATA_BYTES,
                        "directory metadata",
                    )?;
                    directories.insert(directory.to_path_buf());
                    charge(&mut needed, NODE_ALLOCATION, available, "staging space")?;
                }
            }
        }
        if !kind.is_dir() {
            // Payload plus conservative block-rounding/inode overhead. Count
            // repeated file entries too; extraction may reject them later.
            charge(&mut needed, entry.size(), available, "staging space")?;
            charge(&mut needed, NODE_ALLOCATION, available, "staging space")?;
        }
    }
    Ok(())
}

/// Durable stages of a runtime replacement and its recovery.
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum TransactionPhase {
    Staging,
    Committing,
    Committed,
    RollingBack,
}

/// Records a runtime transaction's ownership and recovery phase.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format: u32,
    id: String,
    phase: TransactionPhase,
}

/// Requires a canonical UUID before deriving transaction paths.
fn transaction_id(id: &str) -> Result<()> {
    let parsed = Uuid::parse_str(id).context("Invalid runtime transaction id")?;
    ensure!(
        parsed.to_string() == id,
        "Noncanonical runtime transaction id"
    );
    Ok(())
}

/// Reads a valid transaction journal and rejects symlink staging or backup paths.
fn read_journal(paths: &RuntimePaths) -> Result<Journal> {
    let journal: Journal = read_json(&paths.journal())?;
    ensure!(
        journal.format == 1,
        "Unsupported runtime transaction format"
    );
    transaction_id(&journal.id)?;
    // Journal filenames are derived from a UUID, never accepted as paths.
    for path in [paths.stage(&journal), paths.backup(&journal)] {
        if node_exists(&path)? {
            ensure!(
                !fs::symlink_metadata(path)?.is_symlink(),
                "Runtime transaction path is a symlink"
            );
        }
    }
    Ok(journal)
}

/// Records a new transaction before creating its owned staging directory.
fn start_transaction(paths: &RuntimePaths) -> Result<Journal> {
    fs::create_dir_all(&paths.0)?;
    ensure!(
        !node_exists(&paths.journal())?,
        "Runtime recovery is required first"
    );
    let journal = Journal {
        format: 1,
        id: Uuid::new_v4().to_string(),
        phase: TransactionPhase::Staging,
    };
    ensure!(
        !node_exists(&paths.stage(&journal))? && !node_exists(&paths.backup(&journal))?,
        "Runtime transaction paths already exist"
    );
    // Record ownership before creating anything which might survive a crash.
    atomic_json(&paths.journal(), &journal)?;
    fs::create_dir(paths.stage(&journal))?;
    sync_directory(&paths.0)?;
    Ok(journal)
}

/// Replaces the runtime and tracker, rolling back failures before commit publication.
fn commit_runtime(paths: &RuntimePaths, journal: &Journal) -> Result<()> {
    let staged = paths.stage(journal).join("runtime");
    RuntimeLayout::validate(&staged)?;
    ensure!(
        runtime_version(&staged)?.transaction == journal.id,
        "Staged runtime ownership mismatch"
    );
    if let Ok(runtime) = paths.resolve() {
        wait_for_session(paths, &runtime)?;
    }
    snapshot_legacy_version(paths, journal)?;
    let committing = Journal {
        format: 1,
        id: journal.id.clone(),
        phase: TransactionPhase::Committing,
    };
    atomic_json(&paths.journal(), &committing)?;
    let mut committed_published = false;
    let result = (|| {
        if node_exists(&paths.live())? {
            ensure!(
                !fs::symlink_metadata(paths.live())?.is_symlink(),
                "Refusing to replace a symlink runtime"
            );
            fs::rename(paths.live(), paths.backup(journal)).context("Could not back up runtime")?;
            sync_directory(&paths.0)?;
        }
        fs::rename(&staged, paths.live()).context("Could not activate staged runtime")?;
        sync_directory(&paths.stage(journal))?;
        sync_directory(&paths.0)?;
        paths.sync_legacy_version(&runtime_version(&paths.live())?)?;
        atomic_json_published(
            &paths.journal(),
            &Journal {
                format: 1,
                id: journal.id.clone(),
                phase: TransactionPhase::Committed,
            },
            &mut committed_published,
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        // rename publishes the decision before parent fsync makes it durable.
        // Either surviving journal (Committing or Committed) can roll forward;
        // starting rollback here could contradict a surviving Committed record.
        if committed_published {
            return Err(error).context("Runtime commit publication needs recovery");
        }
        if let Err(rollback) = rollback_runtime(paths, journal) {
            return Err(error).context(format!("Runtime rollback needs recovery: {rollback:#}"));
        }
        return Err(error);
    }
    // Once the compatibility marker is durable the new tree is committed. A
    // cleanup error leaves the journal/remaining backup, not a half-rollback.
    discard_transaction(paths, journal)
        .context("Runtime installed; transaction cleanup needs retry")
}

/// Preserves the legacy tracker's exact bytes or absence for rollback.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackerSnapshot {
    bytes: Option<Vec<u8>>,
}

/// Saves the legacy tracker's current state in transaction-owned staging.
fn snapshot_legacy_version(paths: &RuntimePaths, journal: &Journal) -> Result<()> {
    let components = paths.0.join("components");
    if node_exists(&components)? {
        real_directory(&components, false)?;
    }
    let tracker = components.join("proton.json");
    let bytes = if node_exists(&tracker)? {
        Some(read_metadata(&tracker, MAX_METADATA_BYTES)?)
    } else {
        None
    };
    // Fixed name under UUID-owned staging; neither snapshot nor journal can
    // supply a destination path. Preserve invalid bytes too, never infer v1.
    atomic_json(
        &paths.stage(journal).join(TRACKER_SNAPSHOT),
        &TrackerSnapshot { bytes },
    )
}

/// Durably restores the prior runtime and tracker from an uncommitted transaction.
fn rollback_runtime(paths: &RuntimePaths, journal: &Journal) -> Result<()> {
    let recorded = read_journal(paths)?;
    ensure!(
        recorded.id == journal.id,
        "Runtime transaction ownership changed"
    );
    ensure!(
        recorded.phase != TransactionPhase::Committed,
        "Cannot roll back a committed runtime"
    );
    let snapshot: TrackerSnapshot = serde_json::from_slice(&read_metadata(
        &paths.stage(journal).join(TRACKER_SNAPSHOT),
        MAX_METADATA_BYTES * 4 + 128,
    )?)?;
    ensure!(
        snapshot
            .bytes
            .as_ref()
            .is_none_or(|bytes| bytes.len() as u64 <= MAX_METADATA_BYTES),
        "Runtime tracker snapshot is too large"
    );
    // Persist rollback intent before any move; restart must not bless v2 again
    // if rollback failed before moving the newly installed tree back to staging.
    atomic_json(
        &paths.journal(),
        &Journal {
            format: 1,
            id: journal.id.clone(),
            phase: TransactionPhase::RollingBack,
        },
    )?;
    let staged = paths.stage(journal).join("runtime");
    if node_exists(&paths.live())?
        && runtime_version(&paths.live()).is_ok_and(|marker| marker.transaction == journal.id)
    {
        ensure!(
            !node_exists(&staged)?,
            "Cannot roll back over existing staging"
        );
        if let Ok(runtime) = RuntimeLayout::validate(&paths.live()) {
            wait_for_session(paths, &runtime)?;
        }
        fs::rename(paths.live(), &staged)?;
        sync_directory(&paths.stage(journal))?;
        sync_directory(&paths.0)?;
    }
    if node_exists(&paths.backup(journal))? {
        ensure!(
            !node_exists(&paths.live())?,
            "Recovery refuses to overwrite an unrecognized runtime"
        );
        fs::rename(paths.backup(journal), paths.live())?;
        sync_directory(&paths.0)?;
    }
    let components = paths.0.join("components");
    real_directory(&components, true)?;
    let tracker = components.join("proton.json");
    match snapshot.bytes {
        Some(bytes) => atomic_bytes(&tracker, &bytes, &mut false)?,
        None => {
            if node_exists(&tracker)? {
                ensure!(
                    fs::symlink_metadata(&tracker)?.is_file(),
                    "Invalid legacy tracker"
                );
                fs::remove_file(&tracker)?;
            }
            sync_directory(&components)?;
        }
    }
    // Both old tree and tracker are durable. Cleanup can now discard staging,
    // including the snapshot, even if interrupted before journal deletion.
    atomic_json(
        &paths.journal(),
        &Journal {
            format: 1,
            id: journal.id.clone(),
            phase: TransactionPhase::Staging,
        },
    )?;
    discard_transaction(paths, journal)
}

/// Completes or rolls back an interrupted runtime transaction at the given root.
fn recover_at(paths: &RuntimePaths) -> Result<()> {
    if !node_exists(&paths.journal())? {
        // Unjournaled directories may belong to someone else; no glob cleanup.
        return Ok(());
    }
    let journal = read_journal(paths)?;
    if journal.phase == TransactionPhase::Staging {
        ensure!(
            !node_exists(&paths.backup(&journal))?,
            "Staging transaction unexpectedly has a backup"
        );
        return discard_transaction(paths, &journal);
    }
    let installed =
        runtime_version(&paths.live()).is_ok_and(|marker| marker.transaction == journal.id);
    if journal.phase == TransactionPhase::Committed {
        ensure!(
            installed,
            "Committed runtime marker no longer matches its journal"
        );
        RuntimeLayout::validate(&paths.live())?;
        paths.sync_legacy_version(&runtime_version(&paths.live())?)?;
        return discard_transaction(paths, &journal);
    }
    if journal.phase == TransactionPhase::Committing
        && installed
        && RuntimeLayout::validate(&paths.live()).is_ok()
    {
        match paths.sync_legacy_version(&runtime_version(&paths.live())?) {
            Ok(()) => {
                atomic_json(
                    &paths.journal(),
                    &Journal {
                        format: 1,
                        id: journal.id.clone(),
                        phase: TransactionPhase::Committed,
                    },
                )?;
                return discard_transaction(paths, &journal);
            }
            Err(error) => {
                rollback_runtime(paths, &journal)?;
                return Err(error)
                    .context("Recovered prior runtime because version tracking failed");
            }
        }
    }
    rollback_runtime(paths, &journal)
}

/// Removes a matching transaction's staging and backup before deleting its journal.
fn discard_transaction(paths: &RuntimePaths, journal: &Journal) -> Result<()> {
    let recorded = read_journal(paths)?;
    ensure!(
        recorded.id == journal.id,
        "Runtime transaction ownership changed"
    );
    for path in [paths.backup(journal), paths.stage(journal)] {
        if node_exists(&path)? {
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                !metadata.is_symlink(),
                "Refusing to remove a transaction symlink"
            );
            if metadata.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
    }
    sync_directory(&paths.0)?;
    fs::remove_file(paths.journal())?;
    sync_directory(&paths.0)
}

/// Checks for any filesystem node, including dangling symlinks.
fn node_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Requires a non-symlink directory, optionally creating it when absent.
fn real_directory(path: &Path, create: bool) -> Result<()> {
    if create && !node_exists(path)? {
        fs::create_dir(path)?;
    }
    ensure!(
        fs::symlink_metadata(path)?.is_dir(),
        "Not a real directory: {}",
        path.display()
    );
    Ok(())
}

/// Reads bounded metadata from a regular, non-symlink file.
fn read_metadata(path: &Path, limit: u64) -> Result<Vec<u8>> {
    ensure!(
        fs::symlink_metadata(path)?.is_file(),
        "Not a regular metadata file: {}",
        path.display()
    );
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "Runtime metadata is too large");
    Ok(bytes)
}

/// Deserializes bounded runtime metadata with path-specific errors.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&read_metadata(path, MAX_METADATA_BYTES)?)
        .with_context(|| format!("Invalid runtime metadata {}", path.display()))
}

/// Atomically replaces a JSON metadata file and syncs it to disk.
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    atomic_json_published(path, value, &mut false)
}

/// Writes JSON atomically and reports whether the replacement was published.
fn atomic_json_published(path: &Path, value: &impl Serialize, published: &mut bool) -> Result<()> {
    atomic_bytes(path, &serde_json::to_vec(value)?, published)
}

/// Durably replaces metadata bytes, tracking publication even if the final sync fails.
fn atomic_bytes(path: &Path, bytes: &[u8], published: &mut bool) -> Result<()> {
    *published = false;
    let parent = path.parent().context("Metadata path has no parent")?;
    let temporary = parent.join(format!(".runtime-write-{}", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        #[cfg(test)]
        tests::fail_metadata_at(path, "file-sync")?;
        file.sync_all()?;
        #[cfg(test)]
        tests::fail_metadata_at(path, "rename")?;
        fs::rename(&temporary, path)?;
        *published = true;
        #[cfg(test)]
        tests::fail_metadata_at(path, "parent-sync")?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

/// Syncs a directory's entries to disk.
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?
        .sync_all()
        .with_context(|| format!("Could not sync {}", path.display()))
}

/// Syncs regular files and directories in a runtime tree without following symlinks.
fn sync_tree(root: &Path) -> Result<()> {
    let mut directories = vec![root.to_path_buf()];
    let mut index = 0;
    while index < directories.len() {
        for entry in fs::read_dir(&directories[index])? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                directories.push(entry.path());
            } else if kind.is_file() {
                File::open(entry.path())?.sync_all()?;
            }
        }
        index += 1;
    }
    for directory in directories.into_iter().rev() {
        sync_directory(&directory)?;
    }
    Ok(())
}

/// Supported runtime locations that a session fence can reference.
#[derive(Clone, Copy, Serialize, Deserialize)]
enum SessionRoot {
    Canonical,
    LegacyNested,
    LegacyFlat,
}

impl SessionRoot {
    /// Resolves a recorded runtime location under the application data root.
    fn path(self, paths: &RuntimePaths) -> PathBuf {
        match self {
            Self::Canonical => paths.live(),
            Self::LegacyNested => paths.0.join("proton"),
            Self::LegacyFlat => paths.0.clone(),
        }
    }
}

/// Wine architecture pinned when a session starts.
#[derive(Clone, Copy, Serialize, Deserialize)]
enum SessionWine {
    X86_64,
    Arm64,
}

impl SessionWine {
    /// Returns the binary directory and architecture for the recorded Wine variant.
    fn layout(self) -> (&'static str, &'static str) {
        match self {
            Self::X86_64 => ("files/bin", "x86_64"),
            Self::Arm64 => ("files/bin-arm64", "aarch64"),
        }
    }
}

/// Recorded Proton wrapper identity or evidence that it has exited.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum SessionWrapper {
    #[default]
    Unknown,
    Running {
        pid: u32,
        start_time: u64,
    },
    Reaped,
}

/// Durable session identity that protects the shared prefix until Wine exits.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionFence {
    format: u32,
    root: SessionRoot,
    // Pin the launch-time server variant even if PROTON_USE_ARM64 changes on restart.
    wine: SessionWine,
    boot_id: String,
    #[serde(default)]
    pid_namespace: Option<String>,
    // Older fences cannot prove wrapper exit; treat them as unknown too.
    #[serde(default)]
    wrapper: SessionWrapper,
}

/// Requires a canonical version-4 UUID for a Linux boot identity.
fn validate_boot_id(id: &str) -> Result<()> {
    let uuid = Uuid::parse_str(id).context("Invalid session boot id")?;
    ensure!(
        uuid.to_string() == id
            && uuid.get_version_num() == 4
            && uuid.get_variant() == uuid::Variant::RFC4122,
        "Invalid session boot id"
    );
    Ok(())
}

/// Reads and validates the current Linux boot identity.
fn boot_id() -> Result<String> {
    let bytes = read_metadata(Path::new("/proc/sys/kernel/random/boot_id"), 128)?;
    let id = std::str::from_utf8(&bytes)?.trim().to_owned();
    validate_boot_id(&id)?;
    Ok(id)
}

/// Returns the current process's PID namespace identity.
fn pid_namespace() -> Result<String> {
    Ok(fs::read_link("/proc/self/ns/pid")?
        .to_str()
        .context("Invalid PID namespace identity")?
        .to_owned())
}

/// Rejects mutations whenever a session fence node exists at this root.
fn ensure_no_session_at(paths: &RuntimePaths) -> Result<()> {
    // Any node (including malformed metadata and symlinks) fences mutations.
    ensure!(
        !node_exists(&paths.session())?,
        "A Wine session may still own the shared prefix; session recovery is required"
    );
    Ok(())
}

/// Called by the global operation manager while it owns the profile file lock.
pub(crate) fn ensure_no_session() -> Result<()> {
    ensure_no_session_at(&RuntimePaths::appdata()?)
}

/// Persists a session fence for a known runtime before spawning Proton.
fn fence_session(paths: &RuntimePaths, runtime: &RuntimeLayout) -> Result<SessionFence> {
    ensure_no_session_at(paths)?;
    let root = [
        SessionRoot::Canonical,
        SessionRoot::LegacyNested,
        SessionRoot::LegacyFlat,
    ]
    .into_iter()
    .find(|kind| {
        real_directory(&kind.path(paths), false).is_ok()
            && kind
                .path(paths)
                .canonicalize()
                .is_ok_and(|path| path == runtime.root)
    })
    .context("Session runtime is not a known runtime location")?;
    let wine = if runtime.wineserver == runtime.root.join("files/bin-arm64/wineserver") {
        SessionWine::Arm64
    } else {
        ensure!(
            runtime.wineserver == runtime.root.join("files/bin/wineserver"),
            "Unknown session wineserver"
        );
        SessionWine::X86_64
    };
    let session = SessionFence {
        format: 1,
        root,
        wine,
        boot_id: boot_id()?,
        pid_namespace: Some(pid_namespace()?),
        wrapper: SessionWrapper::Unknown,
    };
    atomic_json(&paths.session(), &session)?;
    Ok(session)
}

/// Persists the wrapper's current identity or exit state in its session fence.
fn record_wrapper(
    paths: &RuntimePaths,
    session: &mut SessionFence,
    wrapper: SessionWrapper,
) -> Result<()> {
    session.wrapper = wrapper;
    atomic_json(&paths.session(), session)
}

/// Process start time and exit state used to recognize a Proton wrapper.
struct WrapperProcess {
    start_time: u64,
    exited: bool,
}

/// Extracts wrapper identity and exit state from Linux process statistics.
fn parse_wrapper_process(pid: u32, stat: &[u8]) -> Result<WrapperProcess> {
    // comm (field 2) may contain spaces, parentheses, or non-UTF8 bytes. Only
    // the final ')' separates it from the ASCII state and numeric fields.
    let end = stat
        .iter()
        .rposition(|byte| *byte == b')')
        .context("Invalid wrapper process stat")?;
    ensure!(
        stat.starts_with(format!("{pid} (").as_bytes()),
        "Wrapper process PID mismatch"
    );
    let mut fields = stat[end + 1..]
        .split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty());
    let state = fields.next().context("Missing wrapper process state")?;
    ensure!(state.len() == 1, "Invalid wrapper process state");
    let start_time = std::str::from_utf8(fields.nth(18).context("Missing wrapper start time")?)?
        .parse::<u64>()
        .context("Invalid wrapper start time")?;
    Ok(WrapperProcess {
        start_time,
        exited: matches!(state[0], b'Z' | b'X' | b'x'),
    })
}

/// Inspects a wrapper PID, returning none only when its process entry is absent.
fn wrapper_process(pid: u32) -> Result<Option<WrapperProcess>> {
    ensure!(pid > 0 && pid <= i32::MAX as u32, "Invalid wrapper PID");
    let path = PathBuf::from(format!("/proc/{pid}/stat"));
    match read_metadata(&path, 4096) {
        Ok(stat) => Ok(Some(parse_wrapper_process(pid, &stat)?)),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            // A missing /proc mount is not evidence that the wrapper exited.
            read_metadata(Path::new("/proc/self/stat"), 4096)?;
            Ok(None)
        }
        Err(error) => Err(error).context("Could not inspect Proton wrapper"),
    }
}

/// Checks whether the recorded wrapper exited, accounting for reused PIDs.
fn wrapper_has_exited(pid: u32, start_time: u64) -> Result<bool> {
    Ok(wrapper_process(pid)?
        .is_none_or(|process| process.start_time != start_time || process.exited))
}

/// Waits for a known wrapper to exit without signalling unrelated processes.
fn wait_for_wrapper(wrapper: SessionWrapper) -> Result<()> {
    match wrapper {
        SessionWrapper::Unknown => {
            bail!("Unknown Proton wrapper identity; session remains fenced until reboot")
        }
        SessionWrapper::Reaped => Ok(()),
        SessionWrapper::Running { pid, start_time } => {
            // No signals or waitpid on a possibly unrelated/reused PID. A
            // zombie/dead state also proves it can no longer start Wine.
            while !wrapper_has_exited(pid, start_time)? {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(())
        }
    }
}

/// Removes a session fence and syncs its parent directory.
fn clear_session(paths: &RuntimePaths) -> Result<()> {
    fs::remove_file(paths.session())?;
    sync_directory(&paths.0)
}

/// Waits for Wine in the recorded prefix before clearing the session fence.
fn quiesce_session(paths: &RuntimePaths, runtime: &RuntimeLayout) -> Result<()> {
    // Unlike an update of a never-used runtime, a recorded session must have a
    // real prefix. A missing/replaced prefix cannot establish quiescence.
    real_directory(&paths.prefix(), false)?;
    if node_exists(&paths.prefix().join("pfx"))? {
        real_directory(&paths.prefix().join("pfx"), false)?;
    }
    wait_for_session(paths, runtime)?;
    clear_session(paths)
}

/// Recovers a fenced session after a reboot or confirmed wrapper and Wine exit.
fn recover_session_at(paths: &RuntimePaths) -> Result<()> {
    if !node_exists(&paths.session())? {
        return Ok(());
    }
    let mut session: SessionFence = read_json(&paths.session())?;
    ensure!(session.format == 1, "Unsupported session fence format");
    validate_boot_id(&session.boot_id)?;
    if session.boot_id != boot_id()? {
        // Linux processes cannot survive a reboot; no runtime execution needed.
        return clear_session(paths);
    }
    ensure!(
        session.pid_namespace.as_deref() == Some(pid_namespace()?.as_str()),
        "Session PID namespace is missing or different; session remains fenced until reboot"
    );
    let (bin, architecture) = session.wine.layout();
    let runtime =
        RuntimeLayout::validate_architecture(&session.root.path(paths), bin, architecture)?;
    wait_for_wrapper(session.wrapper)?;
    record_wrapper(paths, &mut session, SessionWrapper::Reaped)?;
    quiesce_session(paths, &runtime)
}

/// Recovery is the sole operation allowed past a durable session fence. The
/// blocking worker retains its lease/profile lock even if the caller is dropped.
pub async fn recover_session() -> Result<()> {
    recover_session_with(RuntimePaths::appdata()?, operations()).await
}

/// Recovers an existing session while the supplied operation manager holds its lease.
async fn recover_session_with(paths: RuntimePaths, owner: &Operations) -> Result<()> {
    if !node_exists(&paths.session())? {
        return Ok(());
    }
    let operation = owner.begin(OperationKind::Recovery, None)?;
    runtime_worker(operation, move |_| recover_session_at(&paths)).await
}

/// Configures a child's shared-prefix environment without changing the host process.
fn prefix_environment(command: &mut Command, paths: &RuntimePaths) {
    command
        .env("STEAM_COMPAT_DATA_PATH", paths.prefix())
        .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", "")
        .env("WINEPREFIX", paths.prefix().join("pfx"));
}

/// Builds a Proton launch command for an absolute game path and the shared prefix.
fn game_command(
    paths: &RuntimePaths,
    runtime: &RuntimeLayout,
    executable: &Path,
) -> Result<Command> {
    ensure!(
        executable.is_absolute(),
        "Game executable must be an absolute path"
    );
    let executable = executable
        .canonicalize()
        .context("Game executable could not be resolved")?;
    ensure!(executable.is_file(), "Game executable is not a file");
    let root = executable
        .parent()
        .context("Game executable has no parent")?;
    real_directory(&paths.prefix(), true)?;
    if node_exists(&paths.prefix().join("pfx"))? {
        real_directory(&paths.prefix().join("pfx"), false)?;
    }
    let mut command = Command::new(&runtime.executable);
    command.arg("run").arg(&executable).current_dir(root);
    prefix_environment(&mut command, paths);
    Ok(command)
}

/// Reaps an owned child, attempting termination if waiting fails.
fn wait_child(mut child: Child) -> Result<ExitStatus> {
    match child.wait() {
        Ok(status) => Ok(status),
        Err(error) => {
            // Do not leave a live child behind on an unexpected wait failure.
            let _ = child.kill();
            let _ = child.wait();
            Err(error).context("Could not reap runtime process")
        }
    }
}

/// Waits for Wine to exit using the runtime's matching server and libraries.
fn wait_for_session(paths: &RuntimePaths, runtime: &RuntimeLayout) -> Result<()> {
    if !node_exists(&paths.prefix())? {
        return Ok(());
    }
    real_directory(&paths.prefix(), false)?;
    let mut command = Command::new(&runtime.wineserver);
    command.arg("-w").current_dir(&runtime.root);
    prefix_environment(&mut command, paths);
    // Phlogiston 11 uses multiarch library directories; older GE releases use
    // lib/lib64. Match Proton's search order without changing the host process.
    let mut libraries = Vec::new();
    if runtime.wineserver == runtime.root.join("files/bin-arm64/wineserver") {
        libraries.push(runtime.root.join("files/lib/aarch64-linux-gnu"));
    }
    libraries.extend([
        runtime.root.join("files/lib/x86_64-linux-gnu"),
        runtime.root.join("files/lib/i386-linux-gnu"),
        runtime.root.join("files/lib"),
        runtime.root.join("files/lib64"),
    ]);
    if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
        libraries.extend(std::env::split_paths(&existing));
    }
    command.env("LD_LIBRARY_PATH", std::env::join_paths(libraries)?);
    let status = wait_child(
        command
            .spawn()
            .context("Could not start matching wineserver -w")?,
    )?;
    #[cfg(test)]
    tests::fail_metadata_at(&runtime.wineserver, "wait-child")?;
    ensure!(status.success(), "wineserver -w failed: {status}");
    Ok(())
}

/// Launches and supervises a game through Proton under the supplied operation lease.
pub async fn run_game(executable: PathBuf, operation: Operation) -> Result<()> {
    run_game_at(RuntimePaths::appdata()?, executable, operation).await
}

/// Runs game supervision at an explicit root in a lease-owning worker.
async fn run_game_at(paths: RuntimePaths, executable: PathBuf, operation: Operation) -> Result<()> {
    runtime_worker(operation, move |operation| {
        run_game_owned(&paths, &executable, operation)
    })
    .await
}

/// Fences and supervises Proton and Wine, retaining the fence if exit is uncertain.
fn run_game_owned(paths: &RuntimePaths, executable: &Path, operation: &Operation) -> Result<()> {
    operation.check_cancelled()?;
    ensure_no_session_at(paths)?;
    let runtime = paths.resolve()?;
    let mut command = game_command(paths, &runtime, executable)?;
    // Persist before spawn: a crash or any supervision uncertainty must
    // continue excluding other processes after this lease is released.
    let mut fence = fence_session(paths, &runtime)?;
    let mut wrapper_reaped = true;
    let launched = match command.spawn().context("Could not start Proton") {
        Err(error) => Err(error),
        Ok(child) => {
            let pid = child.id();
            // Do not return early on /proc or publication failure: this worker
            // still owns the child and must reap it before releasing its lease.
            let recorded = (|| {
                let process =
                    wrapper_process(pid)?.context("Proton wrapper identity disappeared")?;
                record_wrapper(
                    paths,
                    &mut fence,
                    SessionWrapper::Running {
                        pid,
                        start_time: process.start_time,
                    },
                )
            })();
            let waited = wait_child(child);
            #[cfg(test)]
            let waited = waited.and_then(|status| {
                tests::fail_metadata_at(&runtime.executable, "wait-child")?;
                Ok(status)
            });
            wrapper_reaped = waited.is_ok();
            match (recorded, waited) {
                (Err(error), Err(wait)) => {
                    Err(error).context(format!("Wrapper wait also failed: {wait:#}"))
                }
                (Err(error), _) => Err(error),
                (_, Err(error)) => Err(error),
                (Ok(()), Ok(status)) => {
                    if status.success() {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("Proton exited unsuccessfully: {status}"))
                    }
                }
            }
        }
    };
    if !wrapper_reaped {
        // Even successful -w is premature while this wrapper might start Wine.
        return launched.context("Wrapper exit uncertain; session fence retained");
    }
    // Publish known exit (also after spawn failure) before clearing the fence.
    // Publication errors retain Unknown/Running/Reaped, all safe to recover.
    let session = record_wrapper(paths, &mut fence, SessionWrapper::Reaped)
        .and_then(|_| quiesce_session(paths, &runtime));
    match (launched, session) {
        (Err(error), Err(wait)) => {
            Err(error).context(format!("Wine session wait also failed: {wait:#}"))
        }
        (Err(error), _) => Err(error),
        (_, result) => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::operations::Operations;
    use std::{
        os::unix::fs::symlink,
        sync::atomic::{AtomicUsize, Ordering},
        time::Instant,
    };

    thread_local! {
        static METADATA_FAILURES: std::cell::RefCell<Vec<(PathBuf, &'static str, usize)>> = const {
            std::cell::RefCell::new(Vec::new())
        };
    }

    /// Schedules a one-shot failure after the requested matching boundary visits.
    fn inject_metadata_failure(path: PathBuf, boundary: &'static str, skip: usize) {
        METADATA_FAILURES.with(|failures| failures.borrow_mut().push((path, boundary, skip)));
    }

    /// Triggers a scheduled failure at a matching metadata or process boundary.
    pub(super) fn fail_metadata_at(path: &Path, boundary: &str) -> Result<()> {
        METADATA_FAILURES.with(|failures| {
            let mut failures = failures.borrow_mut();
            if let Some(index) = failures
                .iter()
                .position(|(target, point, _)| target == path && *point == boundary)
            {
                if failures[index].2 == 0 {
                    failures.remove(index);
                    bail!("Injected metadata {boundary} failure: {}", path.display());
                }
                failures[index].2 -= 1;
            }
            Ok(())
        })
    }

    /// Isolated runtime test root that is removed when the fixture is dropped.
    struct Fixture(RuntimePaths);

    impl Fixture {
        /// Creates a unique runtime test directory under application data.
        fn new() -> Self {
            let root = full_path(
                Some(PathBuf::from(format!("runtime-test-{}", Uuid::new_v4()))),
                None,
            )
            .unwrap();
            fs::create_dir_all(&root).unwrap();
            Self(RuntimePaths(root))
        }

        /// Creates a harmless game file with spaces in its path.
        fn game(&self) -> PathBuf {
            let root = self.0.0.join("game with spaces");
            fs::create_dir(&root).unwrap();
            let exe = root.join("Game with spaces.exe");
            fs::write(&exe, b"fixture, never executed directly").unwrap();
            exe
        }

        /// Stages a complete fake runtime with a transaction-owned version marker.
        fn prepared(&self, version: &str) -> Journal {
            let journal = start_transaction(&self.0).unwrap();
            let staged = self.0.stage(&journal).join("runtime");
            layout(&staged);
            marker(&staged, version, &journal.id);
            journal
        }
    }

    impl Drop for Fixture {
        /// Removes this fixture's isolated runtime tree.
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.0);
        }
    }

    /// Writes a fixture file with parent directories and explicit permissions.
    fn write(path: &Path, contents: &[u8], executable: bool) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
        )
        .unwrap();
    }

    /// Creates a complete runtime layout with harmless executable stubs.
    fn layout(root: &Path) {
        for file in [
            "version",
            "filelock.py",
            "utilities.py",
            "protonfixes/__init__.py",
            "files/share/wine/wine.inf",
            "files/lib/wine/x86_64-unix/ntdll.so",
            "files/lib/wine/x86_64-windows/ntdll.dll",
        ] {
            write(&root.join(file), b"fixture", false);
        }
        for file in ["proton", "files/bin/wine", "files/bin/wineserver"] {
            write(&root.join(file), b"#!/bin/sh\nexit 0\n", true);
        }
    }

    /// Writes a valid runtime marker for a fixture version and transaction.
    fn marker(root: &Path, version: &str, transaction: &str) {
        atomic_json(
            &root.join(MARKER),
            &RuntimeVersion {
                format: 1,
                version: version.into(),
                checksum: "a".repeat(64),
                transaction: transaction.into(),
            },
        )
        .unwrap();
    }

    /// Builds valid release metadata using a nonresolving fixture URL.
    fn release(version: &str) -> ComponentRelease {
        ComponentRelease {
            tag: version.into(),
            prerelease: false,
            download: ComponentDownload {
                url: "https://example.invalid/runtime.tar.gz".into(),
                checksum: "a".repeat(64),
            },
        }
    }

    /// Writes a legacy version tracker for the fixture.
    fn legacy(paths: &RuntimePaths, version: &str) {
        fs::create_dir_all(paths.0.join("components")).unwrap();
        atomic_json(
            &paths.0.join("components/proton.json"),
            &LegacyVersion {
                version: version.into(),
            },
        )
        .unwrap();
    }

    /// Creates a small Tokio runtime for asynchronous fixture work.
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    /// Packs a fixture runtime into a gzip-compressed TAR with a top-level directory.
    fn archive(source: &Path, destination: &Path) {
        let encoder = flate2::write::GzEncoder::new(
            File::create(destination).unwrap(),
            flate2::Compression::fast(),
        );
        let mut builder = tar::Builder::new(encoder);
        builder
            .append_dir_all("Phlogiston-fixture", source)
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
    }

    /// Creates a gzip-compressed TAR using caller-supplied entries.
    fn tar_fixture(
        path: &Path,
        fill: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<File>>),
    ) {
        let encoder =
            flate2::write::GzEncoder::new(File::create(path).unwrap(), flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        fill(&mut builder);
        builder.into_inner().unwrap().finish().unwrap();
    }

    /// Builds a checksummed GNU TAR header with the requested entry type and size.
    fn tar_header(kind: u8, size: u64) -> tar::Header {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::new(kind));
        header.set_size(size);
        header.set_mode(0o755);
        header.set_cksum();
        header
    }

    /// Records a fixture transaction phase, snapshotting the tracker before commit.
    fn journal_phase(paths: &RuntimePaths, journal: &Journal, phase: TransactionPhase) {
        if phase == TransactionPhase::Committing {
            snapshot_legacy_version(paths, journal).unwrap();
        }
        atomic_json(
            &paths.journal(),
            &Journal {
                format: 1,
                id: journal.id.clone(),
                phase,
            },
        )
        .unwrap();
    }

    /// Checks the latest-release query and its single, flat metadata response.
    #[test]
    fn deployed_latest_contract_is_single_flat_release() {
        let url = component_url().unwrap();
        let pairs: Vec<_> = url.query_pairs().collect();
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "latest" && value == "true")
        );
        assert!(!pairs.iter().any(|(key, _)| key == "latestOnly"));
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "arch" && value == ARCH)
        );
        let data = serde_json::json!({"tag":"11-0", "prerelease":false,
            "download":{"url":"https://example.invalid/proton.tar.gz", "checksum":"A".repeat(64)}});
        let mut parsed: ComponentRelease = serde_json::from_value(data.clone()).unwrap();
        parsed.validate().unwrap();
        assert_eq!(parsed.tag, "11-0");
        assert_eq!(parsed.download.checksum, "a".repeat(64));
        assert!(serde_json::from_value::<ComponentRelease>(serde_json::json!([data])).is_err());
        assert!(serde_json::from_value::<ComponentRelease>(serde_json::json!({
            "tag":"11-0", "prerelease":false, "download":{"amd64":{"url":"https://example.invalid"}}
        })).is_err());
    }

    /// Rejects prereleases, malformed versions and checksums, and non-HTTPS URLs.
    #[test]
    fn refuses_prerelease_bad_checksums_and_untrusted_urls() {
        let mut data = release("v1");
        data.prerelease = true;
        assert!(data.validate().is_err());
        for checksum in ["", "abc", &"g".repeat(64), &"a".repeat(63)] {
            let mut data = release("v1");
            data.download.checksum = checksum.into();
            assert!(data.validate().is_err());
        }
        for url in [
            "http://example.invalid/runtime",
            "file:///tmp/runtime",
            "not a URL",
        ] {
            let mut data = release("v1");
            data.download.url = url.into();
            assert!(data.validate().is_err());
        }
        for tag in ["", "\n", "v1\nmalformed"] {
            assert!(release(tag).validate().is_err());
        }
    }

    /// Requires a complete runtime with executable launchers and nonempty support files.
    #[test]
    fn runtime_requires_complete_executable_and_wine_layout() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        assert!(paths.resolve().is_err());
        write(&paths.live().join("proton"), b"#!/bin/sh\nexit 0\n", true);
        assert!(paths.resolve().is_err());
        layout(&paths.live());
        assert_eq!(
            paths.resolve().unwrap().executable,
            paths.live().join("proton")
        );
        for file in ["proton", "files/bin/wine", "files/bin/wineserver"] {
            fs::set_permissions(paths.live().join(file), fs::Permissions::from_mode(0o644))
                .unwrap();
            assert!(paths.resolve().is_err(), "{file}");
            fs::set_permissions(paths.live().join(file), fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        fs::remove_file(paths.live().join("files/lib/wine/x86_64-windows/ntdll.dll")).unwrap();
        assert!(paths.resolve().is_err());
        layout(&paths.live());
        fs::write(paths.live().join("files/share/wine/wine.inf"), b"").unwrap();
        assert!(paths.resolve().is_err());
    }

    /// Resolves both legacy layouts without moving files or creating runtime markers.
    #[test]
    fn legacy_runtime_paths_are_supported_without_relocation() {
        for nested in [false, true] {
            let fixture = Fixture::new();
            let paths = &fixture.0;
            let root = if nested {
                paths.0.join("proton")
            } else {
                paths.0.clone()
            };
            layout(&root);
            assert_eq!(paths.resolve().unwrap().executable, root.join("proton"));
            assert!(!paths.live().exists());
            assert!(!paths.journal().exists());
            legacy(paths, "v1");
            assert!(!paths.needs_update(&release("v1")).unwrap());
            assert!(paths.needs_update(&release("v2")).unwrap());
            assert!(!root.join(MARKER).exists());
        }
    }

    /// Accepts runtime-internal symlinks but rejects links outside the distribution.
    #[test]
    fn layout_rejects_external_links_but_accepts_internal_links() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        let wine = paths.live().join("files/bin/wine");
        fs::rename(&wine, paths.live().join("files/bin/wine-real")).unwrap();
        symlink("wine-real", &wine).unwrap();
        assert!(paths.resolve().is_ok());
        fs::remove_file(&wine).unwrap();
        symlink("/bin/false", &wine).unwrap();
        assert!(paths.resolve().is_err());
    }

    /// Ensures a legacy tracker cannot hide missing files or invalid runtime markers.
    #[test]
    fn current_tracker_never_suppresses_runtime_repair() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        legacy(paths, "v2");
        let latest = release("v2");
        assert!(paths.needs_update(&latest).unwrap());
        layout(&paths.live());
        assert!(paths.needs_update(&latest).unwrap());
        marker(&paths.live(), "v1", &Uuid::new_v4().to_string());
        assert!(paths.needs_update(&latest).unwrap());
        marker(&paths.live(), "v2", &Uuid::new_v4().to_string());
        legacy(paths, "v1");
        assert!(!paths.needs_update(&latest).unwrap());
        fs::remove_file(paths.live().join("files/bin/wine")).unwrap();
        assert!(paths.needs_update(&latest).unwrap());
        layout(&paths.live());
        fs::write(paths.live().join(MARKER), b"malformed marker").unwrap();
        assert!(paths.needs_update(&latest).unwrap());
        fs::remove_file(paths.live().join(MARKER)).unwrap();
        fs::write(paths.0.join("components/proton.json"), b"malformed tracker").unwrap();
        assert!(paths.needs_update(&latest).unwrap());
    }

    /// Checks archive checksum validation and the required staging-space budget.
    #[test]
    fn archive_checksum_and_staging_budget_are_enforced() {
        let fixture = Fixture::new();
        let source = fixture.0.0.join("source");
        layout(&source);
        let path = fixture.0.0.join("archive.tar.gz");
        archive(&source, &path);
        verify_archive(&path, &sha256::try_digest(&path).unwrap()).unwrap();
        assert!(verify_archive(&path, &"0".repeat(64)).is_err());
        assert!(verify_archive(&path, "bad hash").is_err());
        assert!(check_archive_space(&path, STORAGE_RESERVE).is_err());
        check_archive_space(&path, STORAGE_RESERVE + 1024 * 1024).unwrap();
    }

    /// Rejects oversized TAR extensions even when they compress to a small archive.
    #[test]
    fn raw_preflight_rejects_compressed_oversized_extensions_before_interpretation() {
        let fixture = Fixture::new();
        let path = fixture.0.0.join("oversized.tar.gz");
        for kind in [b'L', b'K', b'x', b'g'] {
            tar_fixture(&path, |builder| {
                let header = tar_header(kind, MAX_TAR_EXTENSION_BYTES + 1);
                builder
                    .append(
                        &header,
                        std::io::repeat(b'a').take(MAX_TAR_EXTENSION_BYTES + 1),
                    )
                    .unwrap();
            });
            assert!(fs::metadata(&path).unwrap().len() < 2048);
            let error = check_archive_space(&path, u64::MAX).unwrap_err();
            assert!(
                error.to_string().contains("extension is too large"),
                "{error:#}"
            );
        }
    }

    /// Ensures runtime installation rejects oversized extensions before extraction.
    #[test]
    fn extension_preflight_protects_the_actual_extraction_caller() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        let journal = start_transaction(paths).unwrap();
        tar_fixture(&paths.stage(&journal).join("download.tar.gz"), |builder| {
            let header = tar_header(b'L', MAX_TAR_EXTENSION_BYTES + 1);
            builder
                .append(
                    &header,
                    std::io::repeat(0).take(MAX_TAR_EXTENSION_BYTES + 1),
                )
                .unwrap();
        });
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Components, None).unwrap();
        let error = runtime()
            .block_on(install_verified_archive(
                paths,
                &journal,
                &release("v2"),
                &operation,
            ))
            .unwrap_err();
        assert!(error.to_string().contains("extension is too large"));
        assert!(!paths.stage(&journal).join("runtime").exists());
        operation.finish::<()>(Err(error)).unwrap_err();
        recover_at(paths).unwrap();
    }

    /// Counts implicit parent directories once despite explicit or aliased entries.
    #[test]
    fn archive_budget_counts_unique_implicit_parents_and_explicit_aliases() {
        let fixture = Fixture::new();
        let path = fixture.0.0.join("parents.tar.gz");
        tar_fixture(&path, |builder| {
            for name in ["a/b/one", "a/c/two"] {
                builder
                    .append_data(&mut tar_header(b'0', 1), name, &b"x"[..])
                    .unwrap();
            }
            for name in ["a", "./a/b", "a/c"] {
                builder
                    .append_data(&mut tar_header(b'5', 0), name, std::io::empty())
                    .unwrap();
            }
        });
        // Two files and three unique parents, regardless of explicit entries.
        let required = STORAGE_RESERVE + 5 * NODE_ALLOCATION + 2;
        assert!(check_archive_space(&path, required - 1).is_err());
        check_archive_space(&path, required).unwrap();
    }

    /// Accepts ordinary GNU long names and consistent PAX metadata.
    #[test]
    fn normal_longname_and_pax_metadata_remain_supported() {
        let fixture = Fixture::new();
        let path = fixture.0.0.join("metadata.tar.gz");
        tar_fixture(&path, |builder| {
            let name = format!("runtime/{}/file", "long-directory".repeat(10));
            builder
                .append_data(&mut tar_header(b'0', 1), name, &b"x"[..])
                .unwrap();
            builder
                .append_pax_extensions([
                    ("path", b"runtime/pax/file".as_slice()),
                    ("size", b"1".as_slice()),
                ])
                .unwrap();
            builder
                .append_data(&mut tar_header(b'0', 1), "placeholder", &b"x"[..])
                .unwrap();
        });
        check_archive_space(&path, STORAGE_RESERVE + 1024 * 1024).unwrap();
    }

    /// Rejects mismatched PAX sizes and GNU sparse archive entries.
    #[test]
    fn preflight_rejects_pax_offset_divergence_and_sparse_maps() {
        let fixture = Fixture::new();
        let path = fixture.0.0.join("divergent.tar.gz");
        tar_fixture(&path, |builder| {
            builder
                .append_pax_extensions([("size", b"512".as_slice())])
                .unwrap();
            builder
                .append_data(&mut tar_header(b'0', 0), "file", std::io::empty())
                .unwrap();
        });
        assert!(
            check_archive_space(&path, u64::MAX)
                .unwrap_err()
                .to_string()
                .contains("PAX size disagrees")
        );
        tar_fixture(&path, |builder| {
            builder
                .append(&tar_header(b'S', 0), std::io::empty())
                .unwrap();
        });
        assert!(
            check_archive_space(&path, u64::MAX)
                .unwrap_err()
                .to_string()
                .contains("sparse")
        );
        tar_fixture(&path, |builder| {
            builder
                .append_pax_extensions([("GNU.sparse.map", b"0,1".as_slice())])
                .unwrap();
            builder
                .append_data(&mut tar_header(b'0', 1), "file", &b"x"[..])
                .unwrap();
        });
        assert!(
            check_archive_space(&path, u64::MAX)
                .unwrap_err()
                .to_string()
                .contains("Sparse")
        );
    }

    /// Enforces archive path length, depth and decoded-size limits.
    #[test]
    fn archive_limits_bound_depth_path_length_and_decoded_bytes() {
        let fixture = Fixture::new();
        let path = fixture.0.0.join("limits.tar.gz");
        for (name, reason) in [
            (format!("{}file", "a/".repeat(MAX_TAR_DEPTH)), "too deep"),
            ("x".repeat(MAX_TAR_PATH_BYTES + 1), "too long"),
        ] {
            tar_fixture(&path, |builder| {
                builder
                    .append_data(&mut tar_header(b'0', 0), name, std::io::empty())
                    .unwrap();
            });
            assert!(
                check_archive_space(&path, u64::MAX)
                    .unwrap_err()
                    .to_string()
                    .contains(reason)
            );
        }
        // Only a header is needed: reject the claimed size before streaming it.
        let mut encoder = flate2::write::GzEncoder::new(
            File::create(&path).unwrap(),
            flate2::Compression::fast(),
        );
        encoder
            .write_all(tar_header(b'0', MAX_TAR_DECODED_BYTES).as_bytes())
            .unwrap();
        encoder.finish().unwrap();
        assert!(
            check_archive_space(&path, u64::MAX)
                .unwrap_err()
                .to_string()
                .contains("decoded data")
        );
    }

    /// Enforces archive entry-count and cumulative metadata limits.
    #[test]
    fn archive_limits_bound_entry_count_and_aggregate_metadata() {
        let fixture = Fixture::new();
        let path = fixture.0.0.join("many.tar.gz");
        tar_fixture(&path, |builder| {
            let header = tar_header(b'0', 0);
            for _ in 0..=MAX_TAR_ENTRIES {
                builder.append(&header, std::io::empty()).unwrap();
            }
        });
        assert!(
            check_archive_space(&path, u64::MAX)
                .unwrap_err()
                .to_string()
                .contains("Too many")
        );
        tar_fixture(&path, |builder| {
            let header = tar_header(b'L', MAX_TAR_EXTENSION_BYTES);
            for _ in 0..=MAX_TAR_METADATA_BYTES / MAX_TAR_EXTENSION_BYTES {
                builder
                    .append(&header, std::io::repeat(0).take(MAX_TAR_EXTENSION_BYTES))
                    .unwrap();
            }
        });
        assert!(
            raw_tar_preflight(&path)
                .unwrap_err()
                .to_string()
                .contains("metadata")
        );
    }

    /// Checks tracker consistency and cleanup across successive runtime installs.
    #[test]
    fn staged_archive_replaces_v1_with_v2_and_coherent_markers() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        let owner = Operations::default();
        let source = paths.0.join("source");
        layout(&source);
        for version in ["v1", "v2"] {
            fs::write(source.join("version"), version).unwrap();
            let journal = start_transaction(paths).unwrap();
            let downloaded = paths.stage(&journal).join("download.tar.gz");
            archive(&source, &downloaded);
            let mut metadata = release(version);
            metadata.download.checksum = sha256::try_digest(&downloaded).unwrap();
            verify_archive(&downloaded, &metadata.download.checksum).unwrap();
            let operation = owner.begin(OperationKind::Components, None).unwrap();
            let result = runtime().block_on(install_verified_archive(
                paths, &journal, &metadata, &operation,
            ));
            operation.finish(result).unwrap();
            assert_eq!(
                fs::read_to_string(paths.live().join("version")).unwrap(),
                version
            );
            assert_eq!(runtime_version(&paths.live()).unwrap().version, version);
            assert_eq!(
                read_json::<LegacyVersion>(&paths.0.join("components/proton.json"))
                    .unwrap()
                    .version,
                version
            );
            assert!(!paths.journal().exists());
            assert!(!paths.stage(&journal).exists());
            assert!(!paths.backup(&journal).exists());
            assert!(!paths.needs_update(&metadata).unwrap());
        }
    }

    /// Checks that exhausted retries retain the last error and any previous download.
    #[test]
    fn retry_reports_last_concrete_error_without_deleting_previous_file() {
        let fixture = Fixture::new();
        let destination = fixture.0.0.join("existing-archive");
        fs::write(&destination, b"previous download").unwrap();
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Components, None).unwrap();
        let attempts = AtomicUsize::new(0);
        let result = runtime().block_on(retry_download(&operation, || async {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
            bail!("fixture HTTP failure number {attempt}")
        }));
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("fixture HTTP failure number 5"), "{error}");
        assert_eq!(fs::read(destination).unwrap(), b"previous download");
        operation.finish(Ok(())).unwrap();
    }

    /// Recovers coherent runtime and tracker state at each interrupted swap boundary.
    #[test]
    fn recovery_handles_each_rename_interruption() {
        for step in 0..=3 {
            let fixture = Fixture::new();
            let paths = &fixture.0;
            layout(&paths.live());
            marker(&paths.live(), "v1", &Uuid::new_v4().to_string());
            legacy(paths, "v1");
            let journal = fixture.prepared("v2");
            if step >= 1 {
                journal_phase(paths, &journal, TransactionPhase::Committing);
            }
            if step >= 2 {
                fs::rename(paths.live(), paths.backup(&journal)).unwrap();
            }
            if step >= 3 {
                fs::rename(paths.stage(&journal).join("runtime"), paths.live()).unwrap();
            }
            recover_at(paths).unwrap();
            let expected = if step == 3 { "v2" } else { "v1" };
            assert_eq!(
                runtime_version(&paths.live()).unwrap().version,
                expected,
                "step {step}"
            );
            assert_eq!(
                read_json::<LegacyVersion>(&paths.0.join("components/proton.json"))
                    .unwrap()
                    .version,
                expected
            );
            assert!(paths.resolve().is_ok());
            assert!(!paths.journal().exists());
            assert!(!paths.stage(&journal).exists());
            assert!(!paths.backup(&journal).exists());
        }
    }

    /// Restores the prior runtime when the newly activated layout is incomplete.
    #[test]
    fn corrupt_new_layout_rolls_back_to_previous_runtime() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        marker(&paths.live(), "v1", &Uuid::new_v4().to_string());
        let journal = fixture.prepared("v2");
        journal_phase(paths, &journal, TransactionPhase::Committing);
        fs::rename(paths.live(), paths.backup(&journal)).unwrap();
        fs::rename(paths.stage(&journal).join("runtime"), paths.live()).unwrap();
        fs::remove_file(paths.live().join("files/bin/wine")).unwrap();
        recover_at(paths).unwrap();
        assert_eq!(runtime_version(&paths.live()).unwrap().version, "v1");
        assert!(paths.resolve().is_ok());
    }

    /// Rejects a directory at the tracker path before replacing the live runtime.
    #[test]
    fn invalid_tracker_node_is_rejected_before_swap() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        marker(&paths.live(), "v1", &Uuid::new_v4().to_string());
        fs::create_dir_all(paths.0.join("components/proton.json")).unwrap();
        let journal = fixture.prepared("v2");
        assert!(commit_runtime(paths, &journal).is_err());
        assert_eq!(runtime_version(&paths.live()).unwrap().version, "v1");
        assert!(read_journal(paths).unwrap().phase == TransactionPhase::Staging);
        recover_at(paths).unwrap();
        assert!(!paths.journal().exists());
        assert!(paths.0.join("components/proton.json").is_dir());
    }

    /// Distinguishes metadata failures before publication from a failed parent sync.
    #[test]
    fn atomic_metadata_reports_whether_rename_published() {
        for boundary in ["file-sync", "rename", "parent-sync"] {
            let fixture = Fixture::new();
            let file = fixture.0.0.join("metadata");
            fs::write(&file, b"old").unwrap();
            inject_metadata_failure(file.clone(), boundary, 0);
            let mut published = false;
            assert!(atomic_bytes(&file, b"new", &mut published).is_err());
            assert_eq!(published, boundary == "parent-sync");
            assert_eq!(
                fs::read(file).unwrap(),
                if published { b"new" } else { b"old" }
            );
        }
    }

    /// Restores exact legacy tracker bytes or absence when commit publication fails.
    #[test]
    fn final_journal_before_publication_restores_exact_legacy_tracker() {
        for boundary in ["file-sync", "rename"] {
            for prior in [
                Some(b"{ \"version\": \"v1\" }\n".as_slice()),
                Some(b"invalid\xff"),
                None,
            ] {
                for nested in [false, true] {
                    let fixture = Fixture::new();
                    let paths = &fixture.0;
                    let root = if nested {
                        paths.0.join("proton")
                    } else {
                        paths.0.clone()
                    };
                    layout(&root); // No in-tree marker: the tracker must be restored exactly.
                    let tracker = paths.0.join("components/proton.json");
                    if let Some(bytes) = prior {
                        write(&tracker, bytes, false);
                    }
                    let journal = fixture.prepared("v2");
                    inject_metadata_failure(paths.journal(), boundary, 1);
                    assert!(commit_runtime(paths, &journal).is_err());
                    assert!(!paths.live().exists());
                    assert_eq!(fs::read(&tracker).ok().as_deref(), prior);
                    assert!(paths.needs_update(&release("v2")).unwrap());
                    assert!(!paths.journal().exists());
                    assert_eq!(paths.resolve().unwrap().root, root);
                }
            }
        }
    }

    /// Keeps the new runtime when the committed journal was published but not synced.
    #[test]
    fn final_journal_post_rename_failure_never_rolls_back() {
        for surviving_committed in [false, true] {
            let fixture = Fixture::new();
            let paths = &fixture.0;
            layout(&paths.live());
            marker(&paths.live(), "v1", &Uuid::new_v4().to_string());
            legacy(paths, "v1");
            let journal = fixture.prepared("v2");
            inject_metadata_failure(paths.journal(), "parent-sync", 1);
            assert!(commit_runtime(paths, &journal).is_err());
            assert!(read_journal(paths).unwrap().phase == TransactionPhase::Committed);
            assert_eq!(runtime_version(&paths.live()).unwrap().version, "v2");
            assert_eq!(
                runtime_version(&paths.backup(&journal)).unwrap().version,
                "v1"
            );
            if !surviving_committed {
                // A crash before the parent sync may retain the old decision.
                atomic_json(
                    &paths.journal(),
                    &Journal {
                        format: 1,
                        id: journal.id.clone(),
                        phase: TransactionPhase::Committing,
                    },
                )
                .unwrap();
            }
            recover_at(paths).unwrap();
            assert_eq!(runtime_version(&paths.live()).unwrap().version, "v2");
            assert!(!paths.needs_update(&release("v2")).unwrap());
            assert!(!paths.journal().exists());
        }
    }

    /// Resumes interrupted tracker rollback without marking the legacy runtime current.
    #[test]
    fn interrupted_tracker_rollback_restarts_without_blessing_legacy() {
        for prior in [
            Some(b"{\"version\":\"v1\"}".as_slice()),
            Some(b"broken tracker"),
            None,
        ] {
            let fixture = Fixture::new();
            let paths = &fixture.0;
            layout(&paths.0.join("proton"));
            let tracker = paths.0.join("components/proton.json");
            if let Some(bytes) = prior {
                write(&tracker, bytes, false);
            }
            let journal = fixture.prepared("v2");
            inject_metadata_failure(paths.journal(), "rename", 1);
            if prior.is_some() {
                inject_metadata_failure(tracker.clone(), "rename", 1);
            } else {
                // Fail publication of the cleanup decision after restoring absence.
                inject_metadata_failure(paths.journal(), "rename", 1);
            }
            assert!(commit_runtime(paths, &journal).is_err());
            assert!(read_journal(paths).unwrap().phase == TransactionPhase::RollingBack);
            assert!(paths.stage(&journal).join(TRACKER_SNAPSHOT).is_file());
            assert!(!paths.live().exists());
            recover_at(paths).unwrap();
            assert_eq!(fs::read(&tracker).ok().as_deref(), prior);
            assert!(paths.needs_update(&release("v2")).unwrap());
            assert!(!paths.journal().exists());
        }
    }

    /// Rejects oversized legacy metadata before commit and preserves its contents.
    #[test]
    fn oversized_tracker_is_rejected_before_committing() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.0.join("proton"));
        let tracker = paths.0.join("components/proton.json");
        write(
            &tracker,
            &vec![b'x'; MAX_METADATA_BYTES as usize + 1],
            false,
        );
        let journal = fixture.prepared("v2");
        assert!(commit_runtime(paths, &journal).is_err());
        assert!(read_journal(paths).unwrap().phase == TransactionPhase::Staging);
        assert!(!paths.live().exists());
        recover_at(paths).unwrap();
        assert_eq!(fs::metadata(tracker).unwrap().len(), MAX_METADATA_BYTES + 1);
    }

    /// Keeps the prior runtime when the staged version marker cannot be written.
    #[test]
    fn staged_marker_failure_preserves_previous_runtime() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        marker(&paths.live(), "v1", &Uuid::new_v4().to_string());
        let source = paths.0.join("source");
        layout(&source);
        fs::create_dir(source.join(MARKER)).unwrap();
        let journal = start_transaction(paths).unwrap();
        archive(&source, &paths.stage(&journal).join("download.tar.gz"));
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Components, None).unwrap();
        assert!(
            runtime()
                .block_on(install_verified_archive(
                    paths,
                    &journal,
                    &release("v2"),
                    &operation
                ))
                .is_err()
        );
        recover_at(paths).unwrap();
        assert_eq!(runtime_version(&paths.live()).unwrap().version, "v1");
        assert!(!paths.journal().exists());
    }

    /// Retries committed cleanup without rolling back or removing the current runtime.
    #[test]
    fn committed_cleanup_retry_never_rolls_back_or_removes_current_runtime() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        let journal = fixture.prepared("v2");
        fs::rename(paths.stage(&journal).join("runtime"), paths.live()).unwrap();
        journal_phase(paths, &journal, TransactionPhase::Committed);
        fs::create_dir_all(paths.0.join("components/proton.json")).unwrap();
        assert!(recover_at(paths).is_err());
        assert_eq!(runtime_version(&paths.live()).unwrap().version, "v2");
        assert!(paths.journal().is_file());
        fs::remove_dir(paths.0.join("components/proton.json")).unwrap();
        recover_at(paths).unwrap();
        assert!(!paths.journal().exists());
        assert_eq!(runtime_version(&paths.live()).unwrap().version, "v2");
    }

    /// Rejects unsafe transaction paths and leaves unrelated staging trees untouched.
    #[test]
    fn recovery_rejects_unowned_paths_and_never_glob_deletes() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        let unknown = paths
            .0
            .join(format!(".phlogiston-stage-{}", Uuid::new_v4()));
        fs::create_dir(&unknown).unwrap();
        write(&unknown.join("sentinel"), b"keep", false);
        recover_at(paths).unwrap();
        assert!(unknown.join("sentinel").exists());
        for id in ["../outside", "/tmp/outside", "not-a-uuid"] {
            atomic_json(
                &paths.journal(),
                &Journal {
                    format: 1,
                    id: id.into(),
                    phase: TransactionPhase::Staging,
                },
            )
            .unwrap();
            assert!(recover_at(paths).is_err());
            assert!(unknown.join("sentinel").exists());
        }
        fs::remove_file(paths.journal()).unwrap();
        let journal = start_transaction(paths).unwrap();
        fs::remove_dir(paths.stage(&journal)).unwrap();
        symlink(&unknown, paths.stage(&journal)).unwrap();
        assert!(recover_at(paths).is_err());
        assert_eq!(fs::read(unknown.join("sentinel")).unwrap(), b"keep");
    }

    /// Preserves the old runtime and recoverable staging when its Wine wait fails.
    #[test]
    fn failed_replacement_wait_keeps_old_runtime_and_journal_recoverable() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        marker(&paths.live(), "v1", &Uuid::new_v4().to_string());
        fs::create_dir(paths.prefix()).unwrap();
        write(
            &paths.live().join("files/bin/wineserver"),
            b"#!/bin/sh\nexec /bin/false\n",
            true,
        );
        let journal = fixture.prepared("v2");
        assert!(commit_runtime(paths, &journal).is_err());
        assert_eq!(runtime_version(&paths.live()).unwrap().version, "v1");
        recover_at(paths).unwrap();
        assert!(!paths.journal().exists());
    }

    /// Waits with the old runtime's Wine server before activating its replacement.
    #[test]
    fn replacement_wait_uses_the_old_matching_wineserver() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        marker(&paths.live(), "v1", &Uuid::new_v4().to_string());
        fs::create_dir(paths.prefix()).unwrap();
        write(
            &paths.live().join("files/bin/wineserver"),
            b"#!/bin/sh\nprintf '%s\\n' \"$1\" > \"$STEAM_COMPAT_DATA_PATH/old-wait\"\n",
            true,
        );
        let journal = fixture.prepared("v2");
        commit_runtime(paths, &journal).unwrap();
        assert_eq!(
            fs::read_to_string(paths.prefix().join("old-wait")).unwrap(),
            "-w\n"
        );
        assert_eq!(runtime_version(&paths.live()).unwrap().version, "v2");
    }

    /// Checks launch arguments, CWD, child-only environment and process reaping.
    #[test]
    fn launch_records_arguments_cwd_child_environment_and_reaps() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        write(
            &paths.live().join("proton"),
            br##"#!/bin/sh
printf '%s\n' "$@" > "$STEAM_COMPAT_DATA_PATH/args"
pwd > "$STEAM_COMPAT_DATA_PATH/cwd"
printf '%s\n' "$STEAM_COMPAT_CLIENT_INSTALL_PATH" "$WINEPREFIX" > "$STEAM_COMPAT_DATA_PATH/env"
printf '%s\n' "$$" > "$STEAM_COMPAT_DATA_PATH/pid"
"##,
            true,
        );
        write(
            &paths.live().join("files/bin/wineserver"),
            br##"#!/bin/sh
printf '%s\n' "$1" "$WINEPREFIX" > "$STEAM_COMPAT_DATA_PATH/wait"
printf '%s\n' "$$" > "$STEAM_COMPAT_DATA_PATH/server-pid"
"##,
            true,
        );
        let before: Vec<_> = [
            "STEAM_COMPAT_DATA_PATH",
            "STEAM_COMPAT_CLIENT_INSTALL_PATH",
            "WINEPREFIX",
        ]
        .into_iter()
        .map(|key| (key, std::env::var_os(key)))
        .collect();
        let executable = fixture.game();
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Launch, None).unwrap();
        runtime()
            .block_on(run_game_at(
                RuntimePaths(paths.0.clone()),
                executable.clone(),
                operation,
            ))
            .unwrap();
        assert_eq!(
            fs::read_to_string(paths.prefix().join("args")).unwrap(),
            format!("run\n{}\n", executable.display())
        );
        assert_eq!(
            fs::read_to_string(paths.prefix().join("cwd"))
                .unwrap()
                .trim(),
            executable.parent().unwrap().to_str().unwrap()
        );
        assert_eq!(
            fs::read_to_string(paths.prefix().join("env")).unwrap(),
            format!("\n{}\n", paths.prefix().join("pfx").display())
        );
        assert_eq!(
            fs::read_to_string(paths.prefix().join("wait")).unwrap(),
            format!("-w\n{}\n", paths.prefix().join("pfx").display())
        );
        for (key, value) in before {
            assert_eq!(std::env::var_os(key), value);
        }
        for file in ["pid", "server-pid"] {
            let pid: i32 = fs::read_to_string(paths.prefix().join(file))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let mut status = 0;
            // These PIDs are our harmless, already-reaped fixture children.
            assert_eq!(
                unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
                -1
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
        }
        assert!(!owner.is_busy().unwrap());
        assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Completed);
        assert!(!paths.session().exists());
    }

    /// Waits for Wine after a wrapper failure and reports the wrapper's exit status.
    #[test]
    fn failed_wrapper_still_waits_for_wine_and_reports_exit() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        write(
            &paths.live().join("proton"),
            b"#!/bin/sh\nexec /bin/false\n",
            true,
        );
        write(
            &paths.live().join("files/bin/wineserver"),
            b"#!/bin/sh\nprintf waited > \"$STEAM_COMPAT_DATA_PATH/waited\"\n",
            true,
        );
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Launch, None).unwrap();
        let error = runtime()
            .block_on(run_game_at(
                RuntimePaths(paths.0.clone()),
                fixture.game(),
                operation,
            ))
            .unwrap_err();
        assert!(format!("{error:#}").contains("exit status: 1"));
        assert!(paths.prefix().join("waited").is_file());
        assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Failed);
    }

    /// Reports Wine wait failures and leaves the shared prefix fenced.
    #[test]
    fn failed_wineserver_is_an_observable_launch_failure() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        write(
            &paths.live().join("files/bin/wineserver"),
            b"#!/bin/sh\nexec /bin/false\n",
            true,
        );
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Launch, None).unwrap();
        let error = runtime()
            .block_on(run_game_at(
                RuntimePaths(paths.0.clone()),
                fixture.game(),
                operation,
            ))
            .unwrap_err();
        assert!(format!("{error:#}").contains("wineserver -w failed"));
        assert!(!owner.is_busy().unwrap());
        assert!(paths.session().is_file());
        assert!(ensure_no_session_at(paths).is_err());
    }

    /// Prevents launch after fence publication errors and retains any published fence.
    #[test]
    fn session_fence_publication_failure_never_spawns_proton() {
        for boundary in ["file-sync", "rename", "parent-sync"] {
            let fixture = Fixture::new();
            let paths = &fixture.0;
            layout(&paths.live());
            write(
                &paths.live().join("proton"),
                b"#!/bin/sh\nprintf bad > \"$STEAM_COMPAT_DATA_PATH/spawned\"\n",
                true,
            );
            let executable = fixture.game();
            let owner = Operations::default();
            let operation = owner.begin(OperationKind::Launch, None).unwrap();
            inject_metadata_failure(paths.session(), boundary, 0);
            let result = run_game_owned(paths, &executable, &operation);
            assert!(operation.finish(result).is_err());
            assert!(!paths.prefix().join("spawned").exists());
            assert_eq!(
                ensure_no_session_at(paths).is_err(),
                boundary == "parent-sync"
            );
            if boundary == "parent-sync" {
                // No wrapper was spawned, but a later process cannot know
                // that from the pre-spawn record; Unknown must fail closed.
                assert!(recover_session_at(paths).is_err());
                assert!(ensure_no_session_at(paths).is_err());
            } else {
                recover_session_at(paths).unwrap();
                assert!(!paths.session().exists());
            }
        }
    }

    /// Retains the fence through uncertain process waits until recovery proves exit.
    #[test]
    fn wrapper_or_wineserver_wait_uncertainty_retains_fence() {
        for executable in ["proton", "files/bin/wineserver"] {
            let fixture = Fixture::new();
            let paths = &fixture.0;
            layout(&paths.live());
            let owner = Operations::default();
            let operation = owner.begin(OperationKind::Launch, None).unwrap();
            inject_metadata_failure(paths.live().join(executable), "wait-child", 0);
            let result = run_game_owned(paths, &fixture.game(), &operation);
            assert!(operation.finish(result).is_err());
            assert!(ensure_no_session_at(paths).is_err());
            // Repeated recovery uncertainty also cannot clear it.
            inject_metadata_failure(paths.live().join("files/bin/wineserver"), "wait-child", 0);
            assert!(recover_session_at(paths).is_err());
            assert!(ensure_no_session_at(paths).is_err());
            recover_session_at(paths).unwrap();
            ensure_no_session_at(paths).unwrap();
        }
    }

    /// Handles unusual process names, exited states and reused wrapper PIDs.
    #[test]
    fn wrapper_identity_parsing_handles_comm_and_pid_reuse() {
        for state in [b'R', b'Z', b'X', b'x'] {
            let mut stat = b"123 (wrapper ) space \xff) ".to_vec();
            stat.push(state);
            for _ in 4..22 {
                stat.extend_from_slice(b" 0");
            }
            stat.extend_from_slice(b" 98765 0\n");
            let process = parse_wrapper_process(123, &stat).unwrap();
            assert_eq!(process.start_time, 98765);
            assert_eq!(process.exited, state != b'R');
            assert!(parse_wrapper_process(124, &stat).is_err());
        }
        assert!(parse_wrapper_process(123, b"123 (short) R 0").is_err());
        assert!(wrapper_process(0).is_err());
        assert!(wrapper_process(u32::MAX).is_err());
        let pid = std::process::id();
        let process = wrapper_process(pid).unwrap().unwrap();
        assert!(!wrapper_has_exited(pid, process.start_time).unwrap());
        // A live process with the same PID but a different birth time is not
        // the recorded wrapper. Do not wait for or signal that unrelated process.
        assert!(wrapper_has_exited(pid, process.start_time + 1).unwrap());
        wait_for_wrapper(SessionWrapper::Running {
            pid,
            start_time: process.start_time + 1,
        })
        .unwrap();
    }

    /// Keeps unknown or old-format wrapper identities fenced until a different boot.
    #[test]
    fn unknown_and_old_session_fences_cannot_run_waiter_or_clear_on_same_boot() {
        for old_marker in [false, true] {
            let fixture = Fixture::new();
            let paths = &fixture.0;
            layout(&paths.live());
            fs::create_dir(paths.prefix()).unwrap();
            write(
                &paths.live().join("files/bin/wineserver"),
                b"#!/bin/sh\nprintf early > \"$STEAM_COMPAT_DATA_PATH/waited\"\n",
                true,
            );
            let mut fence = fence_session(paths, &paths.resolve().unwrap()).unwrap();
            if old_marker {
                let mut value = serde_json::to_value(&fence).unwrap();
                value.as_object_mut().unwrap().remove("wrapper");
                atomic_json(&paths.session(), &value).unwrap();
            }
            let owner = Operations::default();
            let error = runtime()
                .block_on(recover_session_with(RuntimePaths(paths.0.clone()), &owner))
                .unwrap_err();
            assert!(error.to_string().contains("Unknown Proton wrapper"));
            assert!(ensure_no_session_at(paths).is_err());
            assert!(!paths.prefix().join("waited").exists());
            assert!(!owner.is_busy().unwrap());
            fence.boot_id = Uuid::new_v4().to_string();
            atomic_json(&paths.session(), &fence).unwrap();
            recover_session_at(paths).unwrap();
            ensure_no_session_at(paths).unwrap();
            assert!(!paths.prefix().join("waited").exists());
        }
    }

    /// Keeps recovery leased through wrapper and Wine exit after the caller drops.
    #[test]
    fn recovery_waits_for_initializing_wrapper_then_wine_even_when_caller_drops() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        write(
            &paths.live().join("proton"),
            br##"#!/bin/sh
printf started > "$STEAM_COMPAT_DATA_PATH/wrapper-started"
count=0
while [ ! -f "$STEAM_COMPAT_DATA_PATH/release-wrapper" ]; do
    count=$((count + 1)); [ "$count" -lt 500 ] || exit 1
    sleep 0.01
done
printf started > "$STEAM_COMPAT_DATA_PATH/wine-started"
exit 0
"##,
            true,
        );
        write(
            &paths.live().join("files/bin/wineserver"),
            br##"#!/bin/sh
if [ ! -f "$STEAM_COMPAT_DATA_PATH/wine-started" ]; then
    printf early > "$STEAM_COMPAT_DATA_PATH/waiter-too-early"
    exit 0
fi
printf started > "$STEAM_COMPAT_DATA_PATH/waiter-started"
count=0
while [ ! -f "$STEAM_COMPAT_DATA_PATH/release-wine" ]; do
    count=$((count + 1)); [ "$count" -lt 500 ] || exit 1
    sleep 0.01
done
"##,
            true,
        );
        let layout = paths.resolve().unwrap();
        let mut command = game_command(paths, &layout, &fixture.game()).unwrap();
        let mut fence = fence_session(paths, &layout).unwrap();
        // Model a wrapper surviving its launcher. Keep the Child only so this
        // fixture can reap it later; recovery must work without waitpid ownership.
        let child = command.spawn().unwrap();
        let pid = child.id();
        let start_time = wrapper_process(pid).unwrap().unwrap().start_time;
        record_wrapper(
            paths,
            &mut fence,
            SessionWrapper::Running { pid, start_time },
        )
        .unwrap();
        let owner = Operations::default();
        runtime().block_on(async {
            until_file(&paths.prefix().join("wrapper-started")).await;
            let (started, entered) = tokio::sync::oneshot::channel();
            let worker_paths = RuntimePaths(paths.0.clone());
            let operation = owner.begin(OperationKind::Recovery, None).unwrap();
            let mut task = tokio::spawn(runtime_worker(operation, move |_| {
                started.send(()).unwrap();
                recover_session_at(&worker_paths)
            }));
            entered.await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(150), &mut task)
                    .await
                    .is_err()
            );
            assert!(!paths.prefix().join("waiter-too-early").exists());
            assert!(!paths.prefix().join("waiter-started").exists());
            assert!(ensure_no_session_at(paths).is_err());
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(owner.is_busy().unwrap());
            assert!(owner.begin(OperationKind::Launch, None).is_err());
            fs::write(paths.prefix().join("release-wrapper"), b"go").unwrap();
            until_file(&paths.prefix().join("waiter-started")).await;
            // Recovery must recognize this un-reaped zombie as exited.
            assert!(wrapper_process(pid).unwrap().unwrap().exited);
            assert!(owner.is_busy().unwrap());
            assert!(owner.begin(OperationKind::Components, None).is_err());
            assert!(ensure_no_session_at(paths).is_err());
            fs::write(paths.prefix().join("release-wine"), b"go").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while owner.is_busy().unwrap() {
                assert!(Instant::now() < deadline, "wrapper recovery did not finish");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            ensure_no_session_at(paths).unwrap();
            assert!(!paths.prefix().join("waiter-too-early").exists());
            assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Completed);
        });
        assert!(wait_child(child).unwrap().success());
    }

    /// Checks state-write failures do not skip wrapper reaping before the Wine wait.
    #[test]
    fn wrapper_state_publication_errors_reap_owned_child_before_any_quiescence() {
        for boundary in ["file-sync", "rename", "parent-sync"] {
            for skip in [1, 2] {
                // Running identity, then Reaped decision.
                let fixture = Fixture::new();
                let paths = &fixture.0;
                layout(&paths.live());
                write(
                    &paths.live().join("proton"),
                    br##"#!/bin/sh
printf '%s' "$$" > "$STEAM_COMPAT_DATA_PATH/wrapper-pid"
sleep 0.02
printf done > "$STEAM_COMPAT_DATA_PATH/wrapper-done"
"##,
                    true,
                );
                write(
                    &paths.live().join("files/bin/wineserver"),
                    br##"#!/bin/sh
[ -f "$STEAM_COMPAT_DATA_PATH/wrapper-done" ] || exit 1
printf waited > "$STEAM_COMPAT_DATA_PATH/waited"
"##,
                    true,
                );
                let owner = Operations::default();
                let operation = owner.begin(OperationKind::Launch, None).unwrap();
                inject_metadata_failure(paths.session(), boundary, skip);
                let result = run_game_owned(paths, &fixture.game(), &operation);
                assert!(operation.finish(result).is_err());
                assert!(paths.prefix().join("wrapper-done").exists());
                let pid: i32 = fs::read_to_string(paths.prefix().join("wrapper-pid"))
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut status = 0;
                assert_eq!(
                    unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
                    -1
                );
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::ECHILD)
                );
                if skip == 1 {
                    // Identity publication failed, but owning/reaping the child
                    // still permits publishing Reaped and safely waiting for Wine.
                    assert!(paths.prefix().join("waited").exists());
                    ensure_no_session_at(paths).unwrap();
                } else {
                    assert!(!paths.prefix().join("waited").exists());
                    assert!(ensure_no_session_at(paths).is_err());
                    let fence: SessionFence = read_json(&paths.session()).unwrap();
                    assert_eq!(
                        matches!(fence.wrapper, SessionWrapper::Reaped),
                        boundary == "parent-sync"
                    );
                    recover_session_at(paths).unwrap();
                    assert!(paths.prefix().join("waited").exists());
                    ensure_no_session_at(paths).unwrap();
                }
            }
        }
    }

    /// Keeps an unknown wrapper fenced when neither identity nor exit was confirmed.
    #[test]
    fn unpublished_wrapper_identity_and_uncertain_wait_require_reboot() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        write(
            &paths.live().join("files/bin/wineserver"),
            b"#!/bin/sh\nprintf early > \"$STEAM_COMPAT_DATA_PATH/waited\"\n",
            true,
        );
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Launch, None).unwrap();
        inject_metadata_failure(paths.session(), "rename", 1);
        inject_metadata_failure(paths.live().join("proton"), "wait-child", 0);
        let result = run_game_owned(paths, &fixture.game(), &operation);
        assert!(operation.finish(result).is_err());
        assert!(!paths.prefix().join("waited").exists());
        let fence: SessionFence = read_json(&paths.session()).unwrap();
        assert!(matches!(fence.wrapper, SessionWrapper::Unknown));
        assert!(recover_session_at(paths).is_err());
        assert!(ensure_no_session_at(paths).is_err());
        assert!(!paths.prefix().join("waited").exists());
    }

    /// Rejects malformed session metadata and symlinked or unrecognized runtime paths.
    #[test]
    fn recovery_rejects_untrusted_session_metadata_and_runtime_paths() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        let current_boot = boot_id().unwrap();
        for value in [
            serde_json::json!({"format":1, "root":"/tmp/arbitrary", "wine":"X86_64", "boot_id":current_boot}),
            serde_json::json!({"format":1, "root":"Canonical", "wine":"X86_64", "boot_id":"not-a-boot-id"}),
            serde_json::json!({"format":1, "root":"Canonical", "wine":"X86_64", "boot_id":current_boot, "executable":"/bin/true"}),
            serde_json::json!({"format":2, "root":"Canonical", "wine":"X86_64", "boot_id":current_boot}),
        ] {
            atomic_json(&paths.session(), &value).unwrap();
            assert!(recover_session_at(paths).is_err());
            assert!(ensure_no_session_at(paths).is_err());
        }
        fs::remove_file(paths.session()).unwrap();
        symlink("missing", paths.session()).unwrap();
        assert!(ensure_no_session_at(paths).is_err());
        assert!(recover_session_at(paths).is_err());
        fs::remove_file(paths.session()).unwrap();
        let outside = paths.0.join("not-a-runtime-root");
        layout(&outside);
        symlink(&outside, paths.live()).unwrap();
        atomic_json(
            &paths.session(),
            &SessionFence {
                format: 1,
                root: SessionRoot::Canonical,
                wine: SessionWine::X86_64,
                boot_id: current_boot,
                pid_namespace: Some(pid_namespace().unwrap()),
                wrapper: SessionWrapper::Reaped,
            },
        )
        .unwrap();
        assert!(recover_session_at(paths).is_err());
        assert!(ensure_no_session_at(paths).is_err());
    }

    /// Refuses same-boot recovery when the recorded PID namespace is absent or different.
    #[test]
    fn same_boot_requires_the_recorded_pid_namespace_before_recovery() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        for namespace in [None, Some("pid:[different]".to_owned())] {
            atomic_json(
                &paths.session(),
                &SessionFence {
                    format: 1,
                    root: SessionRoot::Canonical,
                    wine: SessionWine::X86_64,
                    boot_id: boot_id().unwrap(),
                    pid_namespace: namespace,
                    wrapper: SessionWrapper::Reaped,
                },
            )
            .unwrap();
            let error = recover_session_at(paths).unwrap_err();
            assert!(error.to_string().contains("PID namespace"));
            assert!(ensure_no_session_at(paths).is_err());
        }
    }

    /// Clears a previous boot's fence without requiring a runtime or prefix.
    #[test]
    fn different_valid_boot_clears_fence_without_executing_runtime() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        let previous_boot = Uuid::new_v4().to_string();
        assert_ne!(previous_boot, boot_id().unwrap());
        atomic_json(
            &paths.session(),
            &SessionFence {
                format: 1,
                root: SessionRoot::LegacyFlat,
                wine: SessionWine::X86_64,
                boot_id: previous_boot,
                pid_namespace: None,
                wrapper: SessionWrapper::Unknown,
            },
        )
        .unwrap();
        // No runtime or prefix exists. A reboot is sufficient proof.
        recover_session_at(paths).unwrap();
        ensure_no_session_at(paths).unwrap();
    }

    /// Uses the recorded legacy runtime's Wine server even when a newer runtime exists.
    #[test]
    fn recovery_waits_for_the_recorded_legacy_root_not_newer_canonical() {
        for kind in [SessionRoot::LegacyNested, SessionRoot::LegacyFlat] {
            let fixture = Fixture::new();
            let paths = &fixture.0;
            let root = kind.path(paths);
            layout(&root);
            fs::create_dir(paths.prefix()).unwrap();
            let mut fence = fence_session(paths, &RuntimeLayout::validate(&root).unwrap()).unwrap();
            record_wrapper(paths, &mut fence, SessionWrapper::Reaped).unwrap();
            layout(&paths.live());
            write(
                &paths.live().join("files/bin/wineserver"),
                b"#!/bin/sh\nexit 1\n",
                true,
            );
            write(
                &root.join("files/bin/wineserver"),
                b"#!/bin/sh\nprintf waited > \"$STEAM_COMPAT_DATA_PATH/legacy-waited\"\n",
                true,
            );
            recover_session_at(paths).unwrap();
            assert!(paths.prefix().join("legacy-waited").exists());
            ensure_no_session_at(paths).unwrap();
        }
    }

    /// Retains the fence when the prefix is missing or the Wine waiter cannot start.
    #[test]
    fn missing_prefix_and_failed_waiter_spawn_keep_session_fenced() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        let mut fence = fence_session(paths, &paths.resolve().unwrap()).unwrap();
        record_wrapper(paths, &mut fence, SessionWrapper::Reaped).unwrap();
        assert!(recover_session_at(paths).is_err());
        assert!(paths.session().exists());
        fs::create_dir(paths.prefix()).unwrap();
        write(
            &paths.live().join("files/bin/wineserver"),
            b"#!/nonexistent-fixture-interpreter\n",
            true,
        );
        assert!(recover_session_at(paths).is_err());
        assert!(ensure_no_session_at(paths).is_err());
        write(
            &paths.live().join("files/bin/wineserver"),
            b"#!/bin/sh\nexit 0\n",
            true,
        );
        recover_session_at(paths).unwrap();
        ensure_no_session_at(paths).unwrap();
    }

    /// Leaves the active operation untouched when there is no session to recover.
    #[test]
    fn absent_session_recovery_does_not_take_a_recovery_lease() {
        let fixture = Fixture::new();
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Components, None).unwrap();
        runtime()
            .block_on(recover_session_with(
                RuntimePaths(fixture.0.0.clone()),
                &owner,
            ))
            .unwrap();
        assert_eq!(
            owner.status().unwrap().unwrap().kind,
            OperationKind::Components
        );
        operation.finish(Ok(())).unwrap();
    }

    /// Checks Wine descendants stay fenced after failed waits or dropped recovery.
    #[test]
    fn surviving_descendant_and_failed_waiter_remain_fenced_through_recovery() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        write(
            &paths.live().join("proton"),
            br##"#!/bin/sh
/bin/sh -c '
    printf started > "$STEAM_COMPAT_DATA_PATH/descendant-started"
    count=0
    while [ ! -f "$STEAM_COMPAT_DATA_PATH/release-descendant" ]; do
        count=$((count + 1)); [ "$count" -lt 1000 ] || break
        sleep 0.01
    done
    printf done > "$STEAM_COMPAT_DATA_PATH/descendant-done"
' &
exit 0
"##,
            true,
        );
        write(
            &paths.live().join("files/bin/wineserver"),
            b"#!/bin/sh\nexit 1\n",
            true,
        );
        let owner = Operations::default();
        let executable = fixture.game();
        runtime().block_on(async {
            let operation = owner.begin(OperationKind::Launch, None).unwrap();
            assert!(
                run_game_at(RuntimePaths(paths.0.clone()), executable.clone(), operation)
                    .await
                    .is_err()
            );
            until_file(&paths.prefix().join("descendant-started")).await;
            assert!(!paths.prefix().join("descendant-done").exists());
            assert!(ensure_no_session_at(paths).is_err());
            assert!(!owner.is_busy().unwrap());
            let operation = owner.begin(OperationKind::Launch, None).unwrap();
            // Even isolated managers cannot use run_game to bypass the fence.
            assert!(
                run_game_at(RuntimePaths(paths.0.clone()), executable, operation)
                    .await
                    .is_err()
            );
            assert!(
                recover_session_with(RuntimePaths(paths.0.clone()), &owner)
                    .await
                    .is_err()
            );
            assert!(paths.session().exists());
            write(
                &paths.live().join("files/bin/wineserver"),
                br##"#!/bin/sh
printf started > "$STEAM_COMPAT_DATA_PATH/recovery-started"
count=0
while [ ! -f "$STEAM_COMPAT_DATA_PATH/descendant-done" ]; do
    count=$((count + 1)); [ "$count" -lt 1000 ] || exit 1
    sleep 0.01
done
"##,
                true,
            );
            let recovery_owner = owner.clone();
            let recovery_paths = RuntimePaths(paths.0.clone());
            let task =
                tokio::spawn(
                    async move { recover_session_with(recovery_paths, &recovery_owner).await },
                );
            until_file(&paths.prefix().join("recovery-started")).await;
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(owner.is_busy().unwrap());
            assert!(owner.begin(OperationKind::Launch, None).is_err());
            assert!(owner.begin(OperationKind::Components, None).is_err());
            assert!(ensure_no_session_at(paths).is_err());
            fs::write(paths.prefix().join("release-descendant"), b"go").unwrap();
            until_file(&paths.prefix().join("descendant-done")).await;
            let deadline = Instant::now() + Duration::from_secs(5);
            while owner.is_busy().unwrap() {
                assert!(Instant::now() < deadline, "recovery did not reap fixture");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            ensure_no_session_at(paths).unwrap();
            assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Completed);
        });
    }

    /// Waits briefly for a fixture signal file, failing if its deadline expires.
    async fn until_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "fixture did not create {}",
                path.display()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Holds the launch lease through wrapper and Wine exit after the caller is dropped.
    #[test]
    fn dropped_launch_waiter_keeps_lease_through_wrapper_and_wine_lifetime() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        write(
            &paths.live().join("proton"),
            br##"#!/bin/sh
printf started > "$STEAM_COMPAT_DATA_PATH/wrapper-started"
count=0
while [ ! -f "$STEAM_COMPAT_DATA_PATH/release-wrapper" ]; do
    count=$((count + 1)); [ "$count" -lt 1000 ] || exit 1
    sleep 0.01
done
"##,
            true,
        );
        write(
            &paths.live().join("files/bin/wineserver"),
            br##"#!/bin/sh
printf started > "$STEAM_COMPAT_DATA_PATH/server-started"
count=0
while [ ! -f "$STEAM_COMPAT_DATA_PATH/release-server" ]; do
    count=$((count + 1)); [ "$count" -lt 1000 ] || exit 1
    sleep 0.01
done
"##,
            true,
        );
        let executable = fixture.game();
        let owner = Operations::default();
        runtime().block_on(async {
            let operation = owner.begin(OperationKind::Launch, None).unwrap();
            let task = tokio::spawn(run_game_at(
                RuntimePaths(paths.0.clone()),
                executable,
                operation,
            ));
            until_file(&paths.prefix().join("wrapper-started")).await;
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(owner.is_busy().unwrap());
            assert!(owner.begin(OperationKind::Components, None).is_err());
            fs::write(paths.prefix().join("release-wrapper"), b"go").unwrap();
            until_file(&paths.prefix().join("server-started")).await;
            assert!(owner.is_busy().unwrap());
            fs::write(paths.prefix().join("release-server"), b"go").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while owner.is_busy().unwrap() {
                assert!(Instant::now() < deadline, "worker did not reap fixture");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Completed);
        });
    }

    /// Holds the component lease through staging and commit after the caller is dropped.
    #[test]
    fn dropped_component_waiter_keeps_lease_until_staging_and_commit_finish() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        let source = paths.0.join("source");
        layout(&source);
        let journal = start_transaction(paths).unwrap();
        let downloaded = paths.stage(&journal).join("download.tar.gz");
        archive(&source, &downloaded);
        let mut metadata = release("v2");
        metadata.download.checksum = sha256::try_digest(&downloaded).unwrap();
        let owner = Operations::default();
        runtime().block_on(async {
            let handle = tokio::runtime::Handle::current();
            let (started, entered) = tokio::sync::oneshot::channel();
            let (release, resume) = std::sync::mpsc::channel();
            let worker_paths = RuntimePaths(paths.0.clone());
            let operation = owner.begin(OperationKind::Components, None).unwrap();
            let task = tokio::spawn(runtime_worker(operation, move |operation| {
                started.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5))?;
                verify_archive(&downloaded, &metadata.download.checksum)?;
                handle.block_on(install_verified_archive(
                    &worker_paths,
                    &journal,
                    &metadata,
                    operation,
                ))
            }));
            entered.await.unwrap();
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(owner.is_busy().unwrap());
            assert!(owner.begin(OperationKind::Launch, None).is_err());
            release.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while owner.is_busy().unwrap() {
                assert!(Instant::now() < deadline, "component worker did not finish");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Completed);
            assert_eq!(runtime_version(&paths.live()).unwrap().version, "v2");
            assert!(!paths.journal().exists());
        });
    }

    /// Waits for Wine after Proton spawn failure and rejects relative game paths.
    #[test]
    fn spawn_failure_still_waits_and_relative_game_paths_are_rejected() {
        let fixture = Fixture::new();
        let paths = &fixture.0;
        layout(&paths.live());
        write(
            &paths.live().join("proton"),
            b"#!/nonexistent-fixture-interpreter\n",
            true,
        );
        write(
            &paths.live().join("files/bin/wineserver"),
            b"#!/bin/sh\nprintf waited > \"$STEAM_COMPAT_DATA_PATH/waited\"\n",
            true,
        );
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Launch, None).unwrap();
        let error = runtime()
            .block_on(run_game_at(
                RuntimePaths(paths.0.clone()),
                fixture.game(),
                operation,
            ))
            .unwrap_err();
        assert!(format!("{error:#}").contains("Could not start Proton"));
        assert!(paths.prefix().join("waited").is_file());
        assert!(game_command(paths, &paths.resolve().unwrap(), Path::new("relative.exe")).is_err());
        assert!(!owner.is_busy().unwrap());
    }

    /// Cancels pending metadata work without allowing the component operation to commit.
    #[test]
    fn component_cancellation_interrupts_metadata_wait_and_prevents_commit() {
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Components, None).unwrap();
        runtime().block_on(async {
            owner.cancel().unwrap();
            let error = cancellable(&operation, std::future::pending::<Result<()>>())
                .await
                .unwrap_err();
            assert!(error.to_string().contains("cancelled"));
            assert!(owner.is_busy().unwrap());
            assert!(operation.set_phase(Phase::Committing).is_err());
            assert!(operation.finish::<()>(Err(error)).is_err());
        });
        assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Cancelled);
    }
}
