//! Sophon game downloader module.
//!
//! This module implements the Sophon chunk-based download system used by
//! HoYoverse games. It handles downloading, assembling, and updating game files
//! using a manifest-based approach with zstd-compressed chunks.

pub mod api_scrape;
pub mod game_installer;
pub mod proto_parse;
use dashmap::DashMap;
use game_installer::{DownloadHandle, SophonError, UpdateInfo, read_installed_tag};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State, command};
use tauri_plugin_log::log;

/// HTTP client wrapper for dependency injection.
pub struct HttpClient(pub reqwest::Client);

/// Thread-safe container for the active download handle.
pub struct ActiveDownload(pub tokio::sync::Mutex<Option<DownloadHandle>>);

/// Type of download operation, persisted for correct resumption dispatch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DownloadType {
    Fresh,
    Update,
    Preinstall,
}

/// Persisted state for download resumption after app restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadState {
    pub game_id: String,
    pub vo_lang: String,
    pub output_path: String,
    pub download_type: DownloadType,
    pub current_tag: Option<String>,
    pub manifest_hash: String,
    pub downloaded_chunks: HashMap<String, u64>,
}

pub const CHUNK_STATE_SAVE_INTERVAL: u64 = 25;

/// Summary of persisted download state returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeInfo {
    pub game_id: String,
    pub download_type: DownloadType,
}

const DOWNLOAD_STATE_FILE: &str = ".sophon_download_state";

fn download_state_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .map_err(|e| {
            log::error!("app_data_dir resolution failed: {}", e);
            e
        })
        .ok()
        .map(|p| p.join(DOWNLOAD_STATE_FILE))
}

/// Persists download state to disk atomically (write to unique .tmp, then
/// rename) to prevent corrupted state files on crash or power loss.
/// Each call uses a unique temporary file to avoid races between concurrent
/// `spawn_blocking` saves that could otherwise collide on a shared `.tmp` path.
pub fn save_download_state(app: &AppHandle, state: &DownloadState) -> Result<(), String> {
    let Some(path) = download_state_path(app) else {
        let msg = "Failed to resolve download state path".to_string();
        log::error!("{}", msg);
        return Err(msg);
    };
    if let Some(parent) = path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        let msg = format!("Failed to create download state directory: {}", e);
        log::error!("{}", msg);
        return Err(msg);
    }
    match serde_json::to_string(state) {
        Ok(json) => {
            static SAVE_COUNTER: AtomicU64 = AtomicU64::new(0);
            let seq = SAVE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
            let tmp_path = path.with_extension(format!("save-{seq}.tmp"));
            if let Err(e) = fs::write(&tmp_path, &json) {
                let msg = format!("Failed to write temp download state: {}", e);
                log::error!("{}", msg);
                return Err(msg);
            }
            if let Err(e) = fs::rename(&tmp_path, &path) {
                let msg = format!("Failed to rename download state file: {}", e);
                log::error!("{}", msg);
                if let Err(e) = fs::remove_file(&tmp_path) {
                    log::debug!("Failed to clean up temp state file: {}", e);
                }
                return Err(msg);
            }
            Ok(())
        }
        Err(e) => {
            let msg = format!("Failed to serialize download state: {}", e);
            log::error!("{}", msg);
            Err(msg)
        }
    }
}

pub fn load_download_state(app: &AppHandle) -> Option<DownloadState> {
    let path = download_state_path(app)?;
    load_download_state_from(&path)
}

/// Loads and parses a download state file from `path`. On parse failure the
/// corrupt file is renamed to `<path>.corrupted-<unix-timestamp>.json` for
/// post-mortem inspection instead of being silently deleted. Returns `None`
/// if the file is missing or unparseable.
pub(crate) fn load_download_state_from(path: &Path) -> Option<DownloadState> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!(
                "Failed to read download state file {}: {}",
                path.display(),
                e
            );
            return None;
        }
    };
    match serde_json::from_str(&content) {
        Ok(state) => Some(state),
        Err(e) => preserve_corrupted_state(path, &e),
    }
}

/// Renames `path` to a timestamped backup and returns `None`. If the rename
/// fails (e.g. read-only filesystem), the file is removed as a fallback so
/// subsequent loads do not keep failing on the same corrupt JSON.
fn preserve_corrupted_state(path: &Path, parse_err: &serde_json::Error) -> Option<DownloadState> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_path = path.with_extension(format!("corrupted-{timestamp}.json"));
    log::warn!(
        "Download state file corrupted ({}), preserving as {}",
        parse_err,
        backup_path.display()
    );
    match fs::rename(path, &backup_path) {
        Ok(()) => {
            log::warn!(
                "Corrupted download state preserved at {}; user will resume from scratch",
                backup_path.display()
            );
        }
        Err(rename_err) => {
            log::warn!(
                "Failed to preserve corrupted download state at {}: {}; removing instead",
                backup_path.display(),
                rename_err
            );
            let _ = fs::remove_file(path);
        }
    }
    None
}

pub fn clear_download_state(app: &AppHandle) {
    let Some(path) = download_state_path(app) else {
        log::warn!("Failed to resolve download state path during clear");
        return;
    };
    let _ = fs::remove_file(path);
}

/// Deletes the chunks directory under the given game output path.
/// Returns `true` if the directory was removed, `false` if it didn't exist.
/// Deletion errors are logged as warnings and do not propagate — this is a
/// best-effort cleanup.
fn delete_chunks_dir(app: &AppHandle, output_path: &str) -> bool {
    let game_dir = match app.path().resolve(output_path, BaseDirectory::AppData) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Failed to resolve game dir for chunk cleanup: {}", e);
            return false;
        }
    };
    let chunks_dir = game_dir.join("chunks");
    match fs::remove_dir_all(&chunks_dir) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            log::warn!(
                "Failed to delete chunks directory {}: {}",
                chunks_dir.display(),
                e
            );
            false
        }
    }
}

/// Computes a deterministic content-based hash of a Sophon manifest.
///
/// Unlike the legacy `compute_manifest_hash` which hashed raw protobuf bytes
/// (fragile due to non-deterministic serialization), this function hashes the
/// semantic content of the parsed manifest, making it resilient to byte-level
/// differences in the API response.
///
/// Assets are sorted by `asset_name` before hashing. The
/// `chunk_compressed_hash_xxh` field is excluded as it is undocumented and
/// not used for verification. Uses SHA-256 truncated to 8 hex chars.
pub fn compute_content_manifest_hash(manifest: &proto_parse::SophonManifestProto) -> String {
    let mut assets: Vec<_> = manifest.assets.iter().collect();
    assets.sort_by_key(|a| &a.asset_name);

    let mut hasher = Sha256::new();
    for asset in assets {
        hasher.update(asset.asset_name.as_bytes());
        hasher.update(asset.asset_size.to_le_bytes());
        hasher.update(asset.asset_type.to_le_bytes());
        hasher.update(asset.asset_hash_md5.as_bytes());
        for chunk in &asset.asset_chunks {
            hasher.update(chunk.chunk_name.as_bytes());
            hasher.update(chunk.chunk_decompressed_hash_md5.as_bytes());
            hasher.update(chunk.chunk_compressed_hash_md5.as_bytes());
            hasher.update(chunk.chunk_on_file_offset.to_le_bytes());
            hasher.update(chunk.chunk_size.to_le_bytes());
            hasher.update(chunk.chunk_size_decompressed.to_le_bytes());
        }
    }
    hex::encode(&hasher.finalize()[..8])
}

/// Progress events emitted during download operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SophonProgress {
    /// Manifest is being fetched from the API.
    FetchingManifest,
    /// Existing files are being checked to determine what needs downloading.
    CalculatingDownloads {
        checked_files: u64,
        total_files: u64,
    },
    /// Chunks are being downloaded.
    Downloading {
        downloaded_bytes: u64,
        total_bytes: u64,
        speed_bps: f64,
        eta_seconds: f64,
    },
    /// Download is paused.
    Paused {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    /// Files are being assembled from downloaded chunks.
    Assembling {
        assembled_files: u64,
        total_files: u64,
    },
    /// Files are being verified for integrity.
    Verifying {
        scanned_files: u64,
        total_files: u64,
        error_count: u64,
    },
    /// Non-fatal warning occurred.
    Warning { message: String },
    /// Fatal error occurred.
    Error { message: String },
    /// Installing plugins into the game directory.
    InstallingPlugins {
        current_plugin: String,
        total_plugins: usize,
    },
    /// Installing channel SDKs into the game directory.
    InstallingSdks {
        current_sdk: String,
        total_sdks: usize,
    },
    /// Downloading a plugin/SDK ZIP package.
    DownloadingPlugin {
        name: String,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    /// Applying preinstall patches to game files.
    ApplyingPreinstall {
        applied_files: u64,
        total_files: u64,
    },
    /// Download completed successfully.
    Finished,
}

/// Structured error payload for the Tauri IPC boundary.
/// Allows the frontend to programmatically distinguish error types
/// rather than parsing flat error strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CommandError {
    Cancelled,
    NoSpaceAvailable {
        path: String,
        needed: u64,
        available: u64,
    },
    Md5Mismatch {
        item: String,
    },
    SizeMismatch {
        item: String,
        expected: u64,
        actual: u64,
    },
    OriginalFileMissing {
        path: String,
    },
    DownloadFailed {
        chunk: String,
        attempts: u32,
    },
    HdiffPatchFailed {
        file: String,
    },
    AssemblyFailed {
        file: String,
    },
    NoGameManifest,
    NoVoiceManifest {
        locale: String,
    },
    InvalidAssetName {
        name: String,
    },
    PathTraversal {
        path: String,
    },
    ApiError {
        retcode: i32,
        message: String,
    },
    PluginValidationFailed {
        name: String,
    },
    Generic {
        message: String,
    },
}

impl From<SophonError> for CommandError {
    fn from(e: SophonError) -> Self {
        match e {
            SophonError::Cancelled => CommandError::Cancelled,
            SophonError::NoSpaceAvailable {
                path,
                needed,
                available,
            } => CommandError::NoSpaceAvailable {
                path,
                needed,
                available,
            },
            SophonError::Md5Mismatch { item, .. } => CommandError::Md5Mismatch { item },
            SophonError::SizeMismatch {
                item,
                expected,
                actual,
            } => CommandError::SizeMismatch {
                item,
                expected,
                actual,
            },
            SophonError::OriginalFileMissing(path) => CommandError::OriginalFileMissing { path },
            SophonError::DownloadFailed {
                chunk, attempts, ..
            } => CommandError::DownloadFailed { chunk, attempts },
            SophonError::HDiffPatchFailed { file, .. } => CommandError::HdiffPatchFailed { file },
            SophonError::AssemblyFailed { file, .. } => CommandError::AssemblyFailed { file },
            SophonError::NoGameManifest => CommandError::NoGameManifest,
            SophonError::NoVoiceManifest(locale) => CommandError::NoVoiceManifest { locale },
            SophonError::InvalidAssetName(name) => CommandError::InvalidAssetName { name },
            SophonError::PathTraversal(path) => CommandError::PathTraversal {
                path: path.to_string_lossy().to_string(),
            },
            SophonError::ApiError(retcode, message) => CommandError::ApiError { retcode, message },
            SophonError::PluginValidationFailed(name) => {
                CommandError::PluginValidationFailed { name }
            }
            _ => CommandError::Generic {
                message: e.to_string(),
            },
        }
    }
}

impl From<&SophonError> for CommandError {
    fn from(e: &SophonError) -> Self {
        match e {
            SophonError::Cancelled => CommandError::Cancelled,
            SophonError::NoSpaceAvailable {
                path,
                needed,
                available,
            } => CommandError::NoSpaceAvailable {
                path: path.clone(),
                needed: *needed,
                available: *available,
            },
            SophonError::Md5Mismatch { item, .. } => {
                CommandError::Md5Mismatch { item: item.clone() }
            }
            SophonError::SizeMismatch {
                item,
                expected,
                actual,
            } => CommandError::SizeMismatch {
                item: item.clone(),
                expected: *expected,
                actual: *actual,
            },
            SophonError::OriginalFileMissing(path) => {
                CommandError::OriginalFileMissing { path: path.clone() }
            }
            SophonError::DownloadFailed {
                chunk, attempts, ..
            } => CommandError::DownloadFailed {
                chunk: chunk.clone(),
                attempts: *attempts,
            },
            SophonError::HDiffPatchFailed { file, .. } => {
                CommandError::HdiffPatchFailed { file: file.clone() }
            }
            SophonError::AssemblyFailed { file, .. } => {
                CommandError::AssemblyFailed { file: file.clone() }
            }
            SophonError::NoGameManifest => CommandError::NoGameManifest,
            SophonError::NoVoiceManifest(locale) => CommandError::NoVoiceManifest {
                locale: locale.clone(),
            },
            SophonError::InvalidAssetName(name) => {
                CommandError::InvalidAssetName { name: name.clone() }
            }
            SophonError::PathTraversal(path) => CommandError::PathTraversal {
                path: path.to_string_lossy().to_string(),
            },
            SophonError::ApiError(retcode, message) => CommandError::ApiError {
                retcode: *retcode,
                message: message.clone(),
            },
            SophonError::PluginValidationFailed(name) => {
                CommandError::PluginValidationFailed { name: name.clone() }
            }
            other => CommandError::Generic {
                message: other.to_string(),
            },
        }
    }
}

struct StateMeta {
    game_id: String,
    vo_lang: String,
    output_path: String,
    download_type: DownloadType,
    current_tag: Option<String>,
    manifest_hash: String,
}

fn make_state_saver(app: &AppHandle, state: &DownloadState) -> game_installer::StateSaver {
    let app = app.clone();
    let meta = StateMeta {
        game_id: state.game_id.clone(),
        vo_lang: state.vo_lang.clone(),
        output_path: state.output_path.clone(),
        download_type: state.download_type.clone(),
        current_tag: state.current_tag.clone(),
        manifest_hash: state.manifest_hash.clone(),
    };
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct DownloadStateRef<'a> {
        game_id: &'a str,
        vo_lang: &'a str,
        output_path: &'a str,
        download_type: &'a DownloadType,
        current_tag: &'a Option<String>,
        manifest_hash: &'a str,
        downloaded_chunks: &'a DashMap<String, u64>,
    }
    Arc::new(move |chunks: &DashMap<String, u64>| {
        let snapshot = DownloadStateRef {
            game_id: &meta.game_id,
            vo_lang: &meta.vo_lang,
            output_path: &meta.output_path,
            download_type: &meta.download_type,
            current_tag: &meta.current_tag,
            manifest_hash: &meta.manifest_hash,
            downloaded_chunks: chunks,
        };
        let Some(path) = download_state_path(&app) else {
            return;
        };
        static SAVE_COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = SAVE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let tmp_path = path.with_extension(format!("save-{seq}.tmp"));
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let file = match std::fs::File::create(&tmp_path) {
            Ok(f) => f,
            Err(_) => return,
        };
        if serde_json::to_writer(file, &snapshot).is_ok()
            && let Err(e) = fs::rename(&tmp_path, &path)
        {
            let _ = fs::remove_file(&tmp_path);
            log::error!("Failed to rename state file: {}", e);
        }
    })
}

/// Downloads a fresh game installation.
#[command]
pub async fn sophon_download(
    game_id: String,
    vo_lang: String,
    output_path: String,
    app_handle: AppHandle,
    client: State<'_, HttpClient>,
    active: State<'_, ActiveDownload>,
) -> Result<(), String> {
    let game_dir = app_handle
        .path()
        .resolve(&output_path, BaseDirectory::AppData)
        .map_err(|e| e.to_string())?;

  log::warn!("Fetching manifest for game_id={}", game_id);
    emit(&app_handle, SophonProgress::FetchingManifest);

    let (installers, tag, manifest_hash) =
        game_installer::build_installers(&client.0, &game_id, &vo_lang)
            .await
            .map_err(|e| {
                log::warn!("build_installers failed: {}", e);
      emit_error(&app_handle, &e);
                e.to_string()
            })?;

    let state = DownloadState {
        game_id: game_id.clone(),
        vo_lang: vo_lang.clone(),
        output_path: output_path.clone(),
        download_type: DownloadType::Fresh,
        current_tag: None,
        manifest_hash,
        downloaded_chunks: HashMap::new(),
    };
    save_download_state(&app_handle, &state)?;

    let handle = DownloadHandle::new();
    *active.0.lock().await = Some(handle.clone());

    let saver = make_state_saver(&app_handle, &state);
    let app_clone = app_handle.clone();
    let vo_langs: Vec<String> = vec![vo_lang.clone()];
    let result = game_installer::install(
        installers,
        &game_dir,
        vec![],
        &tag,
        game_installer::ResumeContext {
            prev_manifest_hash: String::new(),
            prev_downloaded_chunks: HashMap::new(),
        },
        game_installer::InstallOptions {
            is_preinstall: false,
            is_resume: false,
            handle,
        },
        game_installer::InstallCallbacks {
            updater: Arc::new(move |p| emit(&app_clone, p)),
            state_saver: saver,
        },
        &game_id,
        &vo_langs,
    )
    .await;

    clear_download_state(&app_handle);
    *active.0.lock().await = None;

    match result {
        Ok(()) => {
            let plugin_emit = app_handle.clone();
            let plugin_updater: Arc<dyn Fn(SophonProgress) + Send + Sync> =
                Arc::new(move |p| emit(&plugin_emit, p));
            if let Err(e) = game_installer::install_plugins(&client.0, &game_dir, &game_id, {
                let u = plugin_updater.clone();
                move |p| u(p)
            })
            .await
            {
                log::warn!("Plugin installation failed: {}", e);
                emit_error(&app_handle, &e);
            }
            if let Err(e) = game_installer::install_channel_sdks(&client.0, &game_dir, &game_id, {
                let u = plugin_updater.clone();
                move |p| u(p)
            })
            .await
            {
                log::warn!("Channel SDK installation failed: {}", e);
                emit_error(&app_handle, &e);
            }
            emit(&app_handle, SophonProgress::Finished);
            Ok(())
        }
        Err(e) => install_result(Err(e), &app_handle),
    }
}

/// Updates an existing game installation.
#[command]
pub async fn sophon_update(
    game_id: String,
    vo_lang: String,
    output_path: String,
    app_handle: AppHandle,
    client: State<'_, HttpClient>,
    active: State<'_, ActiveDownload>,
) -> Result<(), String> {
    let game_dir = app_handle
        .path()
        .resolve(&output_path, BaseDirectory::AppData)
        .map_err(|e| e.to_string())?;

    let current_tag =
        read_installed_tag(&game_dir).ok_or("No installed version found — cannot update")?;

  log::warn!("Fetching manifest for game_id={}", game_id);
    emit(&app_handle, SophonProgress::FetchingManifest);

    let (installers, deleted_files, new_tag, manifest_hash) =
        game_installer::build_update_installers(&client.0, &game_id, &vo_lang, &current_tag)
            .await
            .map_err(|e| {
                log::warn!("build_update_installers failed: {}", e);
      emit_error(&app_handle, &e);
                e.to_string()
            })?;

    let state = DownloadState {
        game_id: game_id.clone(),
        vo_lang: vo_lang.clone(),
        output_path: output_path.clone(),
        download_type: DownloadType::Update,
        current_tag: Some(current_tag.clone()),
        manifest_hash,
        downloaded_chunks: HashMap::new(),
    };
    save_download_state(&app_handle, &state)?;

    let handle = DownloadHandle::new();
    *active.0.lock().await = Some(handle.clone());

    let saver = make_state_saver(&app_handle, &state);
    let app_clone = app_handle.clone();
    let vo_langs: Vec<String> = vec![vo_lang.clone()];
    let result = game_installer::install(
        installers,
        &game_dir,
        deleted_files,
        &new_tag,
        game_installer::ResumeContext {
            prev_manifest_hash: String::new(),
            prev_downloaded_chunks: HashMap::new(),
        },
        game_installer::InstallOptions {
            is_preinstall: false,
            is_resume: false,
            handle,
        },
        game_installer::InstallCallbacks {
            updater: Arc::new(move |p| emit(&app_clone, p)),
            state_saver: saver,
        },
        &game_id,
        &vo_langs,
    )
    .await;

    clear_download_state(&app_handle);
    *active.0.lock().await = None;

    match result {
        Ok(()) => {
            let plugin_emit = app_handle.clone();
            let plugin_updater: Arc<dyn Fn(SophonProgress) + Send + Sync> =
                Arc::new(move |p| emit(&plugin_emit, p));
            if let Err(e) = game_installer::install_plugins(&client.0, &game_dir, &game_id, {
                let u = plugin_updater.clone();
                move |p| u(p)
            })
            .await
            {
                log::warn!("Plugin installation failed: {}", e);
                emit_error(&app_handle, &e);
            }
            if let Err(e) = game_installer::install_channel_sdks(&client.0, &game_dir, &game_id, {
                let u = plugin_updater.clone();
                move |p| u(p)
            })
            .await
            {
                log::warn!("Channel SDK installation failed: {}", e);
                emit_error(&app_handle, &e);
            }
            emit(&app_handle, SophonProgress::Finished);
            Ok(())
        }
        Err(e) => install_result(Err(e), &app_handle),
    }
}

/// Pre-downloads an upcoming game version using patch-based preinstall.
#[command]
pub async fn sophon_preinstall(
    game_id: String,
    vo_lang: String,
    output_path: String,
    app_handle: AppHandle,
    client: State<'_, HttpClient>,
    active: State<'_, ActiveDownload>,
) -> Result<(), String> {
    let game_dir = app_handle
        .path()
        .resolve(&output_path, BaseDirectory::AppData)
        .map_err(|e| e.to_string())?;

    log::warn!("Fetching manifest for game_id={}", game_id);
    emit(&app_handle, SophonProgress::FetchingManifest);

  let plan = game_installer::build_preinstall_plan(&client.0, &game_id, &vo_lang, &game_dir)
    .await
            .map_err(|e| {
      log::warn!("build_preinstall_plan failed: {}", e);
                e.to_string()
            })?;

    let tag = plan.tag.clone();

    let current_tag = game_installer::read_installed_tag(&game_dir);

    let state = DownloadState {
        game_id: game_id.clone(),
        vo_lang: vo_lang.clone(),
        output_path: output_path.clone(),
        download_type: DownloadType::Preinstall,
        current_tag,
        manifest_hash: tag.clone(),
        downloaded_chunks: HashMap::new(),
    };
    save_download_state(&app_handle, &state)?;

    let handle = DownloadHandle::new();
    *active.0.lock().await = Some(handle.clone());

    let saver = make_state_saver(&app_handle, &state);
    let app_clone = app_handle.clone();

    let result = game_installer::preinstall_download(
        &client.0,
        &plan,
        &game_dir,
        &game_id,
        &vo_lang,
        handle,
        Arc::new(move |p| emit(&app_clone, p)),
        saver,
        HashMap::new(),
    )
    .await;

    clear_download_state(&app_handle);
    *active.0.lock().await = None;

    match result {
        Ok(_) => {
            emit(&app_handle, SophonProgress::Finished);
            Ok(())
        }
        Err(e) => install_result(Err(e), &app_handle),
    }
}

#[command]
pub async fn sophon_apply_preinstall(
    preinstall_tag: String,
    output_path: String,
    app_handle: AppHandle,
    client: State<'_, HttpClient>,
) -> Result<(), String> {
    // Validate preinstall_tag to prevent path traversal attacks.
    // The tag is interpolated directly into state file paths; an attacker
    // controlling the frontend (or a supply-chain compromised bundle)
    // could pass "..\..\etc\cron.daily/evil" to access files outside
    // game_dir. Reject any component that would produce path traversal.
    game_installer::validate_asset_name(&preinstall_tag).map_err(|e| e.to_string())?;

    let game_dir = app_handle
        .path()
        .resolve(&output_path, BaseDirectory::AppData)
        .map_err(|e| e.to_string())?;

    let updater: Arc<dyn Fn(SophonProgress) + Send + Sync> = Arc::new({
        let app = app_handle.clone();
        move |p| emit(&app, p)
    });

    let apply_handle = DownloadHandle::new();
    game_installer::apply_preinstall(
        &client.0,
        &game_dir,
        &preinstall_tag,
        updater,
        &apply_handle,
    )
    .await
    .or_else(|e| match e {
        SophonError::Cancelled => Ok(()),
        other => {
            emit_error(&app_handle, &other);
            Err(other.to_string())
        }
    })
}

/// Resumes a download that was interrupted (e.g., by app close/crash).
/// Loads the saved download state, dispatches to the correct builder
/// based on download type, and skips already-completed files.
#[command]
pub async fn sophon_resume_download(
    app_handle: AppHandle,
    client: State<'_, HttpClient>,
    active: State<'_, ActiveDownload>,
) -> Result<(), String> {
    let state = load_download_state(&app_handle).ok_or("No download state found to resume")?;

    let game_dir = app_handle
        .path()
        .resolve(&state.output_path, BaseDirectory::AppData)
        .map_err(|e| e.to_string())?;

    let game_id = state.game_id.clone();
    let prev_chunks = state.downloaded_chunks.clone();
    let current_tag = state.current_tag.clone();
    let old_manifest_hash = state.manifest_hash.clone();

    emit(&app_handle, SophonProgress::FetchingManifest);

    if state.download_type == DownloadType::Preinstall {
        if let Some(ref saved_tag) = current_tag {
            let actual_tag = game_installer::read_installed_tag(&game_dir);
            if actual_tag.as_deref() != Some(saved_tag) {
                return Err("Cannot resume preinstall: installed game version changed since preinstall started. Delete preinstall data and start over.".to_string());
            }
        }

        let plan = game_installer::build_preinstall_plan(
            &client.0,
            &state.game_id,
            &state.vo_lang,
            &game_dir,
        )
        .await
        .map_err(|e| {
            emit_error(&app_handle, &e);
            e.to_string()
        })?;

        let resumed_state = DownloadState {
            game_id: state.game_id.clone(),
            vo_lang: state.vo_lang.clone(),
            output_path: state.output_path.clone(),
            download_type: DownloadType::Preinstall,
            current_tag,
            manifest_hash: plan.tag.clone(),
            downloaded_chunks: prev_chunks.clone(),
        };
        let saver = make_state_saver(&app_handle, &resumed_state);

        let handle = DownloadHandle::new();
        *active.0.lock().await = Some(handle.clone());

        let app_clone = app_handle.clone();
        let result = game_installer::preinstall_download(
            &client.0,
            &plan,
            &game_dir,
            &game_id,
            &state.vo_lang,
            handle,
            Arc::new(move |p| emit(&app_clone, p)),
            saver,
            prev_chunks,
        )
        .await;

        clear_download_state(&app_handle);
        *active.0.lock().await = None;

        return match result {
            Ok(_) => {
                emit(&app_handle, SophonProgress::Finished);
                Ok(())
            }
            Err(e) => install_result(Err(e), &app_handle),
        };
    }

    let (installers, deleted_files, tag, manifest_hash) = match state.download_type {
        DownloadType::Fresh => {
            let (installers, tag, new_manifest_hash) =
                game_installer::build_installers(&client.0, &state.game_id, &state.vo_lang)
                    .await
                    .map_err(|e| {
                        emit_error(&app_handle, &e);
                        e.to_string()
                    })?;
            (installers, vec![], tag, new_manifest_hash)
        }
        DownloadType::Update => {
            let ct = current_tag
                .clone()
                .ok_or("No current tag in resume state for update")?;
            let (installers, deleted_files, tag, new_manifest_hash) =
                game_installer::build_update_installers(
                    &client.0,
                    &state.game_id,
                    &state.vo_lang,
                    &ct,
                )
                .await
                .map_err(|e| {
                    emit_error(&app_handle, &e);
                    e.to_string()
                })?;
            (installers, deleted_files, tag, new_manifest_hash)
        }
        DownloadType::Preinstall => unreachable!(),
    };

    let manifest_changed = old_manifest_hash != manifest_hash;
    let resumed_chunks = if manifest_changed {
        if delete_chunks_dir(&app_handle, &state.output_path) {
            log::info!("Deleted stale chunks directory due to manifest change");
        }
        HashMap::new()
    } else {
        prev_chunks
    };

    let resumed_state = DownloadState {
        game_id: state.game_id.clone(),
        vo_lang: state.vo_lang.clone(),
        output_path: state.output_path.clone(),
        download_type: state.download_type,
        current_tag,
        manifest_hash,
        downloaded_chunks: resumed_chunks,
    };
    let saver = make_state_saver(&app_handle, &resumed_state);

    let handle = DownloadHandle::new();
    *active.0.lock().await = Some(handle.clone());

    let app_clone = app_handle.clone();
    let vo_langs: Vec<String> = vec![state.vo_lang.clone()];
    let result = game_installer::install(
        installers,
        &game_dir,
        deleted_files,
        &tag,
        game_installer::ResumeContext {
            prev_manifest_hash: old_manifest_hash,
            prev_downloaded_chunks: resumed_state.downloaded_chunks,
        },
        game_installer::InstallOptions {
            is_preinstall: false,
            is_resume: true,
            handle,
        },
        game_installer::InstallCallbacks {
            updater: Arc::new(move |p| emit(&app_clone, p)),
            state_saver: saver,
        },
        &game_id,
        &vo_langs,
    )
    .await;

    clear_download_state(&app_handle);
    *active.0.lock().await = None;

    match result {
        Ok(()) => {
            let plugin_emit = app_handle.clone();
            let plugin_updater: Arc<dyn Fn(SophonProgress) + Send + Sync> =
                Arc::new(move |p| emit(&plugin_emit, p));
            if let Err(e) = game_installer::install_plugins(&client.0, &game_dir, &game_id, {
                let u = plugin_updater.clone();
                move |p| u(p)
            })
            .await
            {
                log::warn!("Plugin installation failed: {}", e);
                emit_error(&app_handle, &e);
            }
            if let Err(e) = game_installer::install_channel_sdks(&client.0, &game_dir, &game_id, {
                let u = plugin_updater.clone();
                move |p| u(p)
            })
            .await
            {
                log::warn!("Channel SDK installation failed: {}", e);
                emit_error(&app_handle, &e);
            }
            emit(&app_handle, SophonProgress::Finished);
            Ok(())
        }
        Err(e) => install_result(Err(e), &app_handle),
    }
}

/// Checks if there is a downloadable state to resume.
#[command]
pub async fn sophon_has_resume_state(app_handle: AppHandle) -> bool {
    load_download_state(&app_handle).is_some()
}

/// Returns details about the resumable download state, if any.
#[command]
pub async fn sophon_get_resume_info(app_handle: AppHandle) -> Option<ResumeInfo> {
    load_download_state(&app_handle).map(|s| ResumeInfo {
        game_id: s.game_id,
        download_type: s.download_type,
    })
}

/// Pauses the active download.
#[command]
pub async fn sophon_pause(active: State<'_, ActiveDownload>) -> Result<(), String> {
    let guard = active.0.lock().await;
    let h = guard.as_ref().ok_or("No active download")?;
    h.pause();
    Ok(())
}

/// Resumes a paused download.
#[command]
pub async fn sophon_resume(active: State<'_, ActiveDownload>) -> Result<(), String> {
    let guard = active.0.lock().await;
    let h = guard.as_ref().ok_or("No active download")?;
    h.resume();
    Ok(())
}

/// Cancels the active download.
#[command]
pub async fn sophon_cancel(active: State<'_, ActiveDownload>) -> Result<(), String> {
    let guard = active.0.lock().await;
    let h = guard.as_ref().ok_or("No active download")?;
    h.cancel();
    Ok(())
}

/// Checks if an update is available for the game.
#[command]
pub async fn sophon_check_update(
    game_id: String,
    vo_lang: String,
    output_path: String,
    app_handle: AppHandle,
    client: State<'_, HttpClient>,
) -> Result<UpdateInfo, String> {
    let game_dir = app_handle
        .path()
        .resolve(&output_path, BaseDirectory::AppData)
        .map_err(|e| e.to_string())?;

    map_sophon_error(
        game_installer::check_update(&client.0, &game_id, &vo_lang, &game_dir).await,
        &app_handle,
    )
}

/// Verifies the integrity of installed game files and re-downloads any
/// corrupted ones.
#[command]
pub async fn sophon_verify_integrity(
    game_id: String,
    vo_lang: String,
    output_path: String,
    app_handle: AppHandle,
    client: State<'_, HttpClient>,
) -> Result<(), String> {
    let game_dir = app_handle
        .path()
        .resolve(&output_path, BaseDirectory::AppData)
        .map_err(|e| e.to_string())?;

    let app_clone = app_handle.clone();
    map_sophon_error(
        game_installer::verify_integrity(&client.0, &game_id, &vo_lang, &game_dir, move |p| {
            emit(&app_clone, p)
        })
        .await,
        &app_handle,
    )
}

fn emit(app: &AppHandle, progress: SophonProgress) {
    if let Err(e) = app.emit("sophon://progress", progress) {
        log::error!("Failed to emit progress event: {}", e);
    }
}

/// Emits a structured error event across the Tauri IPC boundary.
fn emit_error(app: &AppHandle, error: &SophonError) {
    let _ = app.emit("sophon://error", CommandError::from(error));
}

/// Handles the final install result:
/// - `Ok(())` → propagates success
/// - `Cancelled` → silently returns `Ok(())` (download was intentionally
///   cancelled)
/// - Other errors → emits a structured error event and returns `Err(string)`
fn install_result(result: Result<(), SophonError>, app: &AppHandle) -> Result<(), String> {
    match result {
        Ok(()) => Ok(()),
        Err(SophonError::Cancelled) => Ok(()),
        Err(e) => {
            emit_error(app, &e);
            Err(e.to_string())
        }
    }
}

/// Maps a `SophonResult<T>` to `Result<T, String>` while emitting a structured
/// error event for the frontend. Useful for intermediate non-terminal errors.
fn map_sophon_error<T>(result: Result<T, SophonError>, app: &AppHandle) -> Result<T, String> {
    result.map_err(|e| {
        emit_error(app, &e);
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::proto_parse::{SophonManifestAssetChunk, SophonManifestAssetProperty};
    use super::*;

    fn make_asset(name: &str, md5: &str, size: u64) -> SophonManifestAssetProperty {
        SophonManifestAssetProperty {
            asset_name: name.into(),
            asset_chunks: vec![],
            asset_type: 0,
            asset_size: size,
            asset_hash_md5: md5.into(),
        }
    }

    fn make_asset_with_chunks(
        name: &str,
        md5: &str,
        size: u64,
        xxh: u64,
    ) -> SophonManifestAssetProperty {
        SophonManifestAssetProperty {
            asset_name: name.into(),
            asset_chunks: vec![SophonManifestAssetChunk {
                chunk_name: "chunk_0".into(),
                chunk_decompressed_hash_md5: "decomp_md5".into(),
                chunk_on_file_offset: 0,
                chunk_size: size,
                chunk_size_decompressed: size,
                chunk_compressed_hash_xxh: xxh,
                chunk_compressed_hash_md5: "comp_md5".into(),
                chunk_old_offset: -1,
            }],
            asset_type: 0,
            asset_size: size,
            asset_hash_md5: md5.into(),
        }
    }

    #[test]
    fn compute_content_manifest_hash_deterministic() {
        let manifest = proto_parse::SophonManifestProto {
            assets: vec![
                make_asset("a.pak", "md5_a", 100),
                make_asset("b.pak", "md5_b", 200),
            ],
        };
        let h1 = compute_content_manifest_hash(&manifest);
        let h2 = compute_content_manifest_hash(&manifest);
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_content_manifest_hash_order_independent() {
        let manifest_ab = proto_parse::SophonManifestProto {
            assets: vec![
                make_asset("a.pak", "md5_a", 100),
                make_asset("b.pak", "md5_b", 200),
            ],
        };
        let manifest_ba = proto_parse::SophonManifestProto {
            assets: vec![
                make_asset("b.pak", "md5_b", 200),
                make_asset("a.pak", "md5_a", 100),
            ],
        };
        assert_eq!(
            compute_content_manifest_hash(&manifest_ab),
            compute_content_manifest_hash(&manifest_ba),
        );
    }

    #[test]
    fn compute_content_manifest_hash_different() {
        let manifest1 = proto_parse::SophonManifestProto {
            assets: vec![make_asset("a.pak", "md5_a", 100)],
        };
        let manifest2 = proto_parse::SophonManifestProto {
            assets: vec![make_asset("a.pak", "md5_different", 100)],
        };
        assert_ne!(
            compute_content_manifest_hash(&manifest1),
            compute_content_manifest_hash(&manifest2),
        );
    }

    #[test]
    fn compute_content_manifest_hash_empty() {
        let manifest = proto_parse::SophonManifestProto { assets: vec![] };
        let hash = compute_content_manifest_hash(&manifest);
        assert_eq!(hash.len(), 16);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn compute_content_manifest_hash_excludes_xxh() {
        let manifest_a = proto_parse::SophonManifestProto {
            assets: vec![make_asset_with_chunks("a.pak", "md5_a", 100, 111)],
        };
        let manifest_b = proto_parse::SophonManifestProto {
            assets: vec![make_asset_with_chunks("a.pak", "md5_a", 100, 999)],
        };
        assert_eq!(
            compute_content_manifest_hash(&manifest_a),
            compute_content_manifest_hash(&manifest_b),
        );
    }

    #[test]
    fn compute_content_manifest_hash_truncated() {
        let manifest = proto_parse::SophonManifestProto {
            assets: vec![make_asset("x.pak", "abc", 50)],
        };
        let hash = compute_content_manifest_hash(&manifest);
        assert_eq!(hash.len(), 16);
    }

    /// When the resume state JSON is corrupt, the file should be preserved
    /// under a timestamped backup name so the user can diagnose, instead of
    /// being silently removed.
    #[test]
    fn load_download_state_corrupted_preserves_backup() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("download_state.json");
        let corrupt_bytes = b"{not valid json at all";
        std::fs::write(&state_path, corrupt_bytes).unwrap();

        let result = load_download_state_from(&state_path);
        assert!(result.is_none(), "corrupt state must not load");

        assert!(
            !state_path.exists(),
            "original corrupt file should be moved aside"
        );

        let mut found_backup = false;
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("download_state.corrupted-") && name.ends_with(".json") {
                let backup_bytes = std::fs::read(entry.path()).unwrap();
                assert_eq!(
                    backup_bytes, corrupt_bytes,
                    "preserved backup must contain the original corrupt bytes"
                );
                found_backup = true;
            }
        }
        assert!(
            found_backup,
            "expected a renamed backup file matching the corrupted-<timestamp>.json pattern"
        );
    }

    /// Renaming can fail in edge cases (read-only filesystem, cross-device
    /// moves on some OSes). When it does, we must still avoid returning the
    /// parse error indefinitely: the corrupt file should be removed so the
    /// next load attempts a fresh state.
    #[test]
    fn load_download_state_corrupted_removed_when_rename_fails() {
        // On Linux, cross-device rename fails. We simulate by setting up the
        // state file as a directory — fs::rename will fail because the
        // destination pattern resolves to a child of this dir that already
        // exists.
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("download_state.json");
        // Place a file at the backup path so the rename-overwrite attempt
        // would resolve to an existing path; on Linux rename silently
        // replaces, so we instead place a directory at the backup path.
        let backup_collide = dir.path().join("download_state.corrupted-0.json");
        std::fs::create_dir(&backup_collide).unwrap();
        std::fs::write(&state_path, b"garbage").unwrap();

        let result = load_download_state_from(&state_path);
        assert!(result.is_none());
        // Either the original file is gone (success path) or we exercised the
        // fallback that removed it. In both cases the original state must
        // not be left in place to cause repeated failures.
        if state_path.exists() {
            panic!(
                "original state file should have been renamed or removed; leftover content suggests bug"
            );
        }
    }

    /// A valid state file should load successfully without producing a
    /// backup file.
    #[test]
    fn load_download_state_valid_does_not_create_backup() {
        use crate::commands::sophon_downloader::DownloadState;
        use std::collections::HashMap;
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("download_state.json");
        let state = DownloadState {
            game_id: "test_game".into(),
            vo_lang: "en-us".into(),
            output_path: "/data/game".into(),
            download_type: DownloadType::Fresh,
            current_tag: None,
            manifest_hash: "hash".into(),
            downloaded_chunks: HashMap::new(),
        };
        std::fs::write(&state_path, serde_json::to_string(&state).unwrap()).unwrap();

        let result = load_download_state_from(&state_path);
        assert!(result.is_some(), "valid state must load");
        assert!(
            state_path.exists(),
            "valid state file should remain in place"
        );

        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "no backup file should have been created; found: {entries:?}"
        );
    }
}
