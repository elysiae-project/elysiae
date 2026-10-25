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
        fs::{MultiPathOptions, extract_file, full_path},
        operations::{Operation, OperationKind, Operations, Phase, operations},
        sys::get_available_storage,
    },
    util::{
        progress::ProgressEvent,
        web::{download_file_limited, fetch_data},
    },
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

/// Checks whether a complete supported Proton runtime is installed.
pub fn components_installed() -> Result<bool> {
    Ok(RuntimePaths::appdata()?.resolve().is_ok())
}

/// A launch may share a game-download lease only with a complete runtime and
/// no interrupted runtime transaction. This probe never performs recovery.
pub fn runtime_ready_for_launch() -> Result<bool> {
    let paths = RuntimePaths::appdata()?;

    Ok(!node_exists(&paths.journal())? && paths.resolve().is_ok())
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
            let reporter = operation.reporter()?;
            reporter.report(ProgressEvent::Message(
                "Preparing Phlogiston download…".into(),
            ));
            download_file_limited(
                release.download.url.clone(),
                relative_stage.join("download.tar.gz"),
                None,
                Some(Box::new(move |progress| {
                    reporter.report(ProgressEvent::File(progress))
                })),
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
    operation
        .reporter()?
        .report(ProgressEvent::Extracting("download.tar.gz".into()));
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

    operation
        .reporter()?
        .report(ProgressEvent::Message("Validating Phlogiston…".into()));
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
    Ok(get_available_storage(destination.to_str().context("Runtime path is not UTF-8")?)? as u64)
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
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        *published = true;
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

mod session;

pub(crate) use session::ensure_no_session;
use session::wait_for_session;
pub use session::{recover_session, run_game};

#[cfg(test)]
#[path = "proton_manager/tests.rs"]
mod tests;
