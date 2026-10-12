use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, bail, ensure};
use irmin::{Sophon, SophonProgress, game_installer::SophonError};

use crate::{
    core::{
        fs::{BaseDirectory, full_path, write_file},
        game::Game,
        installations,
        operations::{Operation, OperationKind, Phase, operations},
        proton_manager::{components_installed, recover_runtime, run_game, update_all_modules},
    },
    util::{
        cache::{AssetType, get_cached_asset_paths},
        notifications::broadcast_notification,
        runtime,
        settings::{get_option, voice_language},
    },
};

/// Installation and update states used to decide which game actions are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAvailability {
    Updated,
    Preinstall,
    Outdated,
    NotInstalled,
    Incomplete,
    /// An explicitly imported executable has no irmin version metadata. It can
    /// launch, but updating/repairing it must not guess an installed release.
    Untracked,
}

/// Game download actions handled by the shared operation worker.
#[derive(Clone, Copy)]
enum DownloadAction {
    Install,
    Update,
    Repair,
}

/// Progress is best-effort and bounded by the caller's channel. Terminal events
/// replace an older progress item if necessary. The returned Result and
/// operations().status() remain authoritative even if the consumer closes;
/// irmin's Finished event alone is not evidence that an operation succeeded.
pub async fn download_game(
    game: Game,
    lang: &str,
    sender: async_channel::Sender<SophonProgress>,
) -> Result<()> {
    start_download(DownloadAction::Install, game, lang, sender).await
}

/// Downloads a preinstall or applies the game's current update.
pub async fn download_update(
    game: Game,
    lang: &str,
    sender: async_channel::Sender<SophonProgress>,
) -> Result<()> {
    start_download(DownloadAction::Update, game, lang, sender).await
}

/// Runs an integrity scan and attempts repairs for an installed game.
pub async fn repair_game(
    game: Game,
    lang: &str,
    sender: async_channel::Sender<SophonProgress>,
) -> Result<()> {
    start_download(DownloadAction::Repair, game, lang, sender).await
}

/// Starts an owned download worker and reports its authoritative final result.
async fn start_download(
    action: DownloadAction,
    game: Game,
    lang: &str,
    sender: async_channel::Sender<SophonProgress>,
) -> Result<()> {
    game.executable()?; // Unreleased games are explicitly unsupported, not guessed.
    let lang = voice_language(lang)?;
    let kind = match action {
        DownloadAction::Install => OperationKind::Install,
        DownloadAction::Update => OperationKind::Update,
        DownloadAction::Repair => OperationKind::Repair,
    };
    let operation = operations().begin(kind, Some(game))?;
    // Dropping this outer future detaches, rather than aborts, the worker. The
    // worker retains ownership until irmin and all its awaited work have ended.
    run_worker(operation, move |operation| async move {
        let result = execute_download(action, game, lang, sender.clone(), &operation).await;
        let result = operation.finish(result);
        let final_event = match &result {
            Ok(()) => SophonProgress::Finished,
            Err(error) => SophonProgress::Error {
                message: format!("{error:#}"),
            },
        };
        let _ = sender.force_send(final_event);
        result
    })
    .await
}

/// Runs work on the backend runtime, retaining ownership if the caller detaches.
async fn run_worker<F: Future<Output = Result<()>> + Send + 'static>(
    operation: Operation,
    work: impl FnOnce(Operation) -> F,
) -> Result<()> {
    runtime::spawn(work(operation))
        .await
        .context("Backend worker failed")?
}

/// Performs the requested game mutation and verifies installed files as needed.
async fn execute_download(
    action: DownloadAction,
    game: Game,
    lang: &str,
    sender: async_channel::Sender<SophonProgress>,
    operation: &Operation,
) -> Result<()> {
    let path = match action {
        DownloadAction::Install => installations::prepare_install(game, operation)?,
        DownloadAction::Update | DownloadAction::Repair => {
            installations::writable_path(game, operation)?
        }
    };
    validate_mutation_tree(&path, operation)?;
    let sophon = installer(game, lang, path)?;
    match action {
        DownloadAction::Install => {
            mark_pending(sophon.game_dir())?;
            let progress = sender.clone();
            sophon
                .download(operation.handle(), move |event| {
                    forward_progress(&progress, event)
                })
                .await?;
            operation.check_cancelled()?;
            operation.set_phase(Phase::Verifying)?;
            verify(&sophon, sender).await?;
            clear_pending(sophon.game_dir())?;
            if get_option("generate-desktop-shortcut")
                .and_then(bool::try_from)
                .unwrap_or(false)
                && let Err(error) = generate_desktop_file(game)
            {
                log::warn!("Game installed, but shortcut creation failed: {error:#}");
            }
            broadcast_notification("Download Complete");
        }
        DownloadAction::Update => {
            ensure!(
                local_status(game, sophon.game_dir())? != UpdateAvailability::Incomplete,
                "Installation is incomplete; resume installation or repair it first"
            );
            let info = checked_update(&sophon).await?;
            ensure!(
                info.current_tag.is_some(),
                "No installed version; install the game first"
            );
            match classify_update(&info) {
                UpdateAvailability::Outdated => {
                    mark_pending(sophon.game_dir())?;
                    if apply_live_preinstall(&info) {
                        let progress = sender.clone();
                        let result = sophon
                            .apply_preinstall(&info.remote_tag, operation.handle(), move |event| {
                                forward_progress(&progress, event)
                            })
                            .await;
                        match result {
                            Ok(()) => {}
                            // The pinned library requires pre-download metadata after
                            // promotion. This error occurs before it modifies files.
                            Err(SophonError::NoPreinstallAvailable) => {
                                log::warn!(
                                    "Promoted preinstall metadata unavailable; using normal update"
                                );
                                operation.check_cancelled()?;
                                let progress = sender.clone();
                                sophon
                                    .update(operation.handle(), move |event| {
                                        forward_progress(&progress, event)
                                    })
                                    .await?;
                            }
                            Err(error) => return Err(error.into()),
                        }
                    } else {
                        let progress = sender.clone();
                        sophon
                            .update(operation.handle(), move |event| {
                                forward_progress(&progress, event)
                            })
                            .await?;
                    }
                    operation.check_cancelled()?;
                    operation.set_phase(Phase::Verifying)?;
                    verify(&sophon, sender).await?;
                    clear_pending(sophon.game_dir())?;
                    broadcast_notification("Update Complete");
                }
                UpdateAvailability::Preinstall => {
                    sophon
                        .preinstall(operation.handle(), move |event| {
                            forward_progress(&sender, event)
                        })
                        .await?;
                    operation.check_cancelled()?;
                    broadcast_notification("Preinstall Download Complete");
                }
                UpdateAvailability::Updated => {}
                _ => bail!("No complete installation is available to update"),
            }
        }
        DownloadAction::Repair => {
            ensure!(
                installed_tag(sophon.game_dir())?.is_some(),
                "No installed version is available to verify; resume installation first"
            );
            operation.set_phase(Phase::Verifying)?;
            mark_pending(sophon.game_dir())?;
            verify(&sophon, sender).await?;
            clear_pending(sophon.game_dir())?;
            broadcast_notification("Integrity scan complete: no errors reported");
        }
    }
    Ok(())
}

/// Forwards progress without blocking, leaving completion to the operation worker.
fn forward_progress(sender: &async_channel::Sender<SophonProgress>, event: SophonProgress) {
    if !matches!(event, SophonProgress::Finished) {
        let _ = sender.try_send(event);
    }
}

/// Builds a fully verifying installer with the shared HTTP client.
fn installer(game: Game, lang: &str, path: PathBuf) -> Result<Sophon> {
    Ok(Sophon::builder(game.code(), path)
        .vo_lang(lang)
        .client(crate::util::web::http_client()?.clone())
        .verify_mode(irmin::VerifyMode::Full)
        .build())
}

/// Queries game update metadata with a bounded wait.
async fn checked_update(sophon: &Sophon) -> Result<irmin::game_installer::UpdateInfo> {
    // This query has no mutation workers, so dropping it on deadline is safe.
    Ok(
        tokio::time::timeout(std::time::Duration::from_secs(30), sophon.check_update())
            .await
            .context("Game update check timed out")??,
    )
}

/// Reads a path's mount identifier without following its final symlink.
fn mount_id(path: &Path) -> Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let mut stat: libc::statx = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::statx(
            libc::AT_FDCWD,
            path.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW | libc::AT_NO_AUTOMOUNT,
            libc::STATX_MNT_ID,
            &mut stat,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    ensure!(
        stat.stx_mask & libc::STATX_MNT_ID != 0,
        "Filesystem cannot establish installation mount boundaries"
    );
    Ok(stat.stx_mnt_id)
}

/// Irmin writes its own filesystem paths rather than using Elysiae's wrappers.
/// Fail closed on linked/special/mounted entries before handing it a mutable
/// root. Import/launch may retain such layouts read-only. Concurrent external
/// mutation of an owned installation is not supported.
fn validate_mutation_tree(root: &Path, operation: &Operation) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let mount = mount_id(root)?;
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        operation.check_cancelled()?;
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "Game mutation cannot follow symlink {}",
            path.display()
        );
        ensure!(
            mount_id(&path)? == mount,
            "Game mutation cannot cross a mount at {}",
            path.display()
        );
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        } else {
            ensure!(
                metadata.is_file() && metadata.nlink() == 1,
                "Game mutation requires an unshared regular file: {}",
                path.display()
            );
        }
    }
    Ok(())
}

/// Tracks scan completeness and errors independently of upstream completion events.
#[derive(Default)]
struct VerificationReport {
    total: u64,
    scanned: u64,
    errors: u64,
    error: Option<String>,
}

impl VerificationReport {
    /// Records scan progress and errors reported during verification.
    fn observe(&mut self, event: &SophonProgress) {
        match event {
            SophonProgress::Verifying {
                scanned_files,
                total_files,
                error_count,
            } => {
                self.scanned = *scanned_files;
                self.total = *total_files;
                self.errors = self.errors.max(*error_count);
            }
            SophonProgress::Error { message } => {
                self.error = Some(message.clone());
            }
            _ => {}
        }
    }

    /// Accepts only a complete scan that reported no damaged files or errors.
    fn result(&self) -> Result<()> {
        if let Some(error) = &self.error {
            bail!("Integrity scan failed: {error}");
        }
        ensure!(
            self.total > 0 && self.scanned == self.total,
            "Integrity scan did not report a complete manifest"
        );
        ensure!(
            self.errors == 0,
            "Irmin found {} damaged files and attempted repairs, but this library version cannot reliably confirm repair success. Run verification again to check their state.",
            self.errors
        );
        Ok(())
    }
}

/// Runs integrity verification and checks that the reported scan is complete and clean.
async fn verify(sophon: &Sophon, sender: async_channel::Sender<SophonProgress>) -> Result<()> {
    // Verification in the current irmin pin has no control handle. Do not drop
    // its future on cancel: spawned work could continue after releasing ownership.
    // The upstream fixes are explicitly deferred; errors found during the scan
    // must not be disguised by its unconditional Finished/Ok after failed repair.
    let report = Arc::new(Mutex::new(VerificationReport::default()));
    let observed = report.clone();
    sophon
        .verify_integrity(move |event| {
            if let Ok(mut report) = observed.lock() {
                report.observe(&event);
            }
            forward_progress(&sender, event);
        })
        .await?;
    report
        .lock()
        .map_err(|_| anyhow::anyhow!("Verification report lock poisoned"))?
        .result()
}

/// Reads a bounded installed-version marker, treating missing or empty markers as absent.
fn installed_tag(path: &Path) -> Result<Option<String>> {
    use std::io::Read;
    let marker = path.join(".sophon_version");
    let metadata = match std::fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_file(),
        "Installed version marker must be a regular file"
    );
    let mut tag = String::new();
    std::fs::File::open(marker)?
        .take(4097)
        .read_to_string(&mut tag)?;
    ensure!(tag.len() <= 4096, "Installed version marker is too large");
    let tag = tag.trim();
    Ok((!tag.is_empty()).then(|| tag.to_owned()))
}

const PENDING_VERIFICATION: &str = ".elysiae-verification-pending";

/// Checks whether an installation still has a pending-verification marker.
fn mutation_pending(root: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(root.join(PENDING_VERIFICATION)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Durably marks an installation as awaiting successful verification.
fn mark_pending(root: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let path = root.join(PENDING_VERIFICATION);
    let file = match std::fs::File::create_new(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)?
        }
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.nlink() == 1,
        "Invalid verification marker"
    );
    file.sync_all()?;
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}

/// Durably removes the pending marker after successful verification.
fn clear_pending(root: &Path) -> Result<()> {
    std::fs::remove_file(root.join(PENDING_VERIFICATION))?;
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}

/// Classifies local installation markers and the executable without network access.
fn local_status(game: Game, path: &Path) -> Result<UpdateAvailability> {
    if !path.try_exists()? {
        return Ok(UpdateAvailability::NotInstalled);
    }
    ensure!(path.is_dir(), "Game installation path is not a directory");
    if mutation_pending(path)? {
        return Ok(UpdateAvailability::Incomplete);
    }
    let executable = path.join(game.executable()?);
    if !executable.is_file() || installed_tag(path)?.is_none() {
        return Ok(UpdateAvailability::Incomplete);
    }
    ensure!(
        executable.canonicalize()?.starts_with(path.canonicalize()?),
        "Game executable escapes installation directory"
    );
    Ok(UpdateAvailability::Updated)
}

/// Classifies update metadata, prioritizing required updates over future preinstalls.
fn classify_update(info: &irmin::game_installer::UpdateInfo) -> UpdateAvailability {
    if info.current_tag.is_none() {
        UpdateAvailability::Incomplete
    } else if info.update_available {
        UpdateAvailability::Outdated
    } else if info.preinstall_available && !info.preinstall_downloaded {
        UpdateAvailability::Preinstall
    } else {
        UpdateAvailability::Updated
    }
}

/// Checks whether the downloaded preinstall matches the currently available update.
fn apply_live_preinstall(info: &irmin::game_installer::UpdateInfo) -> bool {
    info.update_available
        && info.preinstall_downloaded
        && info.preinstall_tag.as_deref() == Some(info.remote_tag.as_str())
}

/// Gets local installation status, recognizing explicitly imported untracked games.
pub fn local_update_status(game: Game) -> Result<UpdateAvailability> {
    game.executable()?;
    let path = installations::path(game)?;
    let status = local_status(game, &path)?;
    if status == UpdateAvailability::Incomplete
        && !mutation_pending(&path)?
        && installed_tag(&path)?.is_none()
        && installations::registered(game)?.is_some_and(|record| !record.managed)
        && path.join(game.executable()?).is_file()
    {
        ensure!(
            path.join(game.executable()?)
                .canonicalize()?
                .starts_with(&path),
            "Imported executable escapes installation directory"
        );
        return Ok(UpdateAvailability::Untracked);
    }
    Ok(status)
}

/// Combines local installation state with remote update availability when applicable.
pub async fn update_status(game: Game, lang: &str) -> Result<UpdateAvailability> {
    let path = installations::path(game)?;
    let local = local_update_status(game)?;
    if local != UpdateAvailability::Updated {
        return Ok(local);
    }
    let sophon = installer(game, voice_language(lang)?, path)?;
    Ok(classify_update(&checked_update(&sophon).await?))
}

/// Pauses or resumes the active download when its phase permits it.
pub fn toggle_downloading() -> Result<()> {
    operations().toggle_pause()
}
/// Requests cancellation without releasing the active worker's ownership.
pub fn cancel_download() -> Result<()> {
    operations().cancel()
}

/// Checks whether local installation state allows the game to be launched.
pub fn game_installed(game: Game) -> Result<bool> {
    Ok(matches!(
        local_update_status(game)?,
        UpdateAvailability::Updated | UpdateAvailability::Untracked
    ))
}

/// Writes a desktop shortcut that selects the game in Elysiae without launching it.
fn generate_desktop_file(game: Game) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let icon = get_cached_asset_paths(game, "en-us", AssetType::Icon)
        .ok()
        .and_then(|paths| paths.into_iter().next())
        .map(|path| {
            path.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
        })
        .unwrap_or_else(|| "app.elysiae.Elysiae".to_owned());
    let contents = format!(
        "[Desktop Entry]\nName={}\nComment=Play {} with Elysiae\nExec=xdg-open elysiae://open-game/{}\nType=Application\nCategories=Game;\nIcon={icon}\n",
        game.display_name(),
        game.display_name(),
        game.code()
    );
    let path = PathBuf::from(format!("{}.desktop", game.code()));
    write_file(
        path.clone(),
        contents.as_bytes(),
        Some(BaseDirectory::Desktop),
    )?;
    std::fs::set_permissions(
        full_path(Some(path), Some(BaseDirectory::Desktop))?,
        std::fs::Permissions::from_mode(0o755),
    )?;
    Ok(())
}

/// Prepares the runtime and launches an installed game under operation ownership.
pub async fn launch_game(game: Game) -> Result<()> {
    runtime::spawn(async move {
        ensure!(
            game_installed(game)?,
            "No complete or explicitly imported game installation"
        );
        // Recovery is offline and runs before deciding whether a fresh runtime
        // download is necessary. A restored working version is immediately usable.
        recover_runtime().await?;
        if !components_installed()? {
            update_all_modules().await?;
        }
        let operation = operations().begin(OperationKind::Launch, Some(game))?;
        let executable = (|| -> Result<PathBuf> {
            // Resolve again under ownership: a relocation may have completed
            // while the initial component install was running.
            ensure!(
                game_installed(game)?,
                "Game installation changed before launch"
            );
            Ok(installations::path(game)?.join(game.executable()?))
        })();
        match executable {
            Ok(executable) => run_game(executable, operation).await,
            Err(error) => operation.finish(Err(error)),
        }
    })
    .await
    .context("Launch worker failed")?
}

/// Registers an existing installation with the requested modification permission.
pub async fn import_game(game: Game, path: PathBuf, allow_modify: bool) -> Result<()> {
    let operation = operations().begin(OperationKind::Import, Some(game))?;
    runtime::spawn(async move {
        tokio::task::spawn_blocking(move || {
            let result = installations::import(game, &path, allow_modify, &operation);
            operation.finish(result)
        })
        .await
        .context("Import worker failed")?
    })
    .await
    .context("Import task failed")?
}

/// Relocates an installation, optionally deleting its confirmed managed source.
pub async fn relocate_game(
    game: Game,
    destination: PathBuf,
    confirmed_source: PathBuf,
    remove_source: bool,
) -> Result<()> {
    let operation = operations().begin(OperationKind::Relocate, Some(game))?;
    runtime::spawn(async move {
        tokio::task::spawn_blocking(move || {
            let result = installations::relocate(
                game,
                &destination,
                &confirmed_source,
                remove_source,
                &operation,
            );
            operation.finish(result)
        })
        .await
        .context("Relocation worker failed")?
    })
    .await
    .context("Relocation task failed")?
}

/// Unregisters a game, optionally deleting its confirmed managed installation.
pub async fn uninstall_game(game: Game, confirmed_root: PathBuf, delete_files: bool) -> Result<()> {
    let operation = operations().begin(OperationKind::Uninstall, Some(game))?;
    runtime::spawn(async move {
        tokio::task::spawn_blocking(move || {
            let result = installations::uninstall(game, &confirmed_root, delete_files, &operation);
            operation.finish(result)
        })
        .await
        .context("Uninstall worker failed")?
    })
    .await
    .context("Uninstall task failed")?
}

/// Recovers interrupted installation maintenance under exclusive operation ownership.
pub async fn recover_maintenance(game: Game) -> Result<()> {
    let operation = operations().begin(OperationKind::Relocate, Some(game))?;
    runtime::spawn(async move {
        tokio::task::spawn_blocking(move || {
            let result = installations::recover(game, &operation);
            operation.finish(result)
        })
        .await
        .context("Maintenance recovery worker failed")?
    })
    .await
    .context("Maintenance recovery task failed")?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds update metadata with both a current update and a future preinstall.
    fn update_info() -> irmin::game_installer::UpdateInfo {
        irmin::game_installer::UpdateInfo {
            update_available: true,
            preinstall_available: true,
            preinstall_downloaded: false,
            current_tag: Some("v1".into()),
            remote_tag: "v2".into(),
            preinstall_tag: Some("v3".into()),
            update_compressed_size: 0,
            update_decompressed_size: 0,
            preinstall_compressed_size: 0,
            preinstall_decompressed_size: 0,
        }
    }

    /// Checks update priority and release matching for promoted preinstalls.
    #[test]
    fn required_live_updates_precede_future_preinstalls() {
        let mut info = update_info();
        assert_eq!(classify_update(&info), UpdateAvailability::Outdated);
        info.preinstall_downloaded = true;
        assert!(!apply_live_preinstall(&info));
        info.preinstall_tag = Some("v2".into());
        assert!(apply_live_preinstall(&info));
        info.update_available = false;
        assert!(!apply_live_preinstall(&info));
        info.preinstall_downloaded = false;
        assert_eq!(classify_update(&info), UpdateAvailability::Preinstall);
        info.current_tag = None;
        assert_eq!(classify_update(&info), UpdateAvailability::Incomplete);
    }

    /// Checks installation completeness and rejects pending or invalid version markers.
    #[test]
    fn installation_status_requires_verified_metadata() {
        let root = std::env::temp_dir().join(format!("elysiae-status-{}", uuid::Uuid::new_v4()));
        let status = || local_status(Game::Bh3, &root).unwrap();
        assert_eq!(status(), UpdateAvailability::NotInstalled);
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("BH3.exe"), b"fixture").unwrap();
        assert_eq!(status(), UpdateAvailability::Incomplete);
        std::fs::write(root.join(".sophon_version"), b"v1\n").unwrap();
        assert_eq!(status(), UpdateAvailability::Updated);
        mark_pending(&root).unwrap();
        assert_eq!(status(), UpdateAvailability::Incomplete);
        clear_pending(&root).unwrap();
        assert_eq!(status(), UpdateAvailability::Updated);
        std::os::unix::fs::symlink("missing", root.join(PENDING_VERIFICATION)).unwrap();
        assert!(mutation_pending(&root).unwrap());
        assert!(mark_pending(&root).is_err());
        std::fs::remove_file(root.join(PENDING_VERIFICATION)).unwrap();
        std::fs::remove_file(root.join(".sophon_version")).unwrap();
        std::os::unix::fs::symlink("BH3.exe", root.join(".sophon_version")).unwrap();
        assert!(local_status(Game::Bh3, &root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Checks that mutation preflight rejects symbolic and hard links to external files.
    #[test]
    fn mutation_preflight_rejects_links_before_touching_external_files() {
        let root = std::env::temp_dir().join(format!("elysiae-mutation-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("game")).unwrap();
        std::fs::write(root.join("sentinel"), b"keep").unwrap();
        let owner = crate::core::operations::Operations::default();
        let operation = owner.begin(OperationKind::Update, Some(Game::Bh3)).unwrap();
        assert!(validate_mutation_tree(&root.join("game"), &operation).is_ok());
        std::os::unix::fs::symlink("../sentinel", root.join("game/link")).unwrap();
        assert!(validate_mutation_tree(&root.join("game"), &operation).is_err());
        std::fs::remove_file(root.join("game/link")).unwrap();
        std::fs::hard_link(root.join("sentinel"), root.join("game/link")).unwrap();
        assert!(validate_mutation_tree(&root.join("game"), &operation).is_err());
        assert_eq!(std::fs::read(root.join("sentinel")).unwrap(), b"keep");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Checks that upstream completion cannot conceal scan errors or uncertain repairs.
    #[test]
    fn upstream_finished_does_not_override_failed_repairs() {
        let mut report = VerificationReport::default();
        assert!(report.result().is_err());
        report.observe(&SophonProgress::Verifying {
            scanned_files: 2,
            total_files: 2,
            error_count: 1,
        });
        report.observe(&SophonProgress::Finished);
        assert!(
            report
                .result()
                .unwrap_err()
                .to_string()
                .contains("cannot reliably confirm")
        );
        let mut clean = VerificationReport::default();
        clean.observe(&SophonProgress::Verifying {
            scanned_files: 2,
            total_files: 2,
            error_count: 0,
        });
        assert!(clean.result().is_ok());
        clean.observe(&SophonProgress::Error {
            message: "repair failed".into(),
        });
        assert!(clean.result().is_err());
    }
}
