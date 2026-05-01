//! Sophon game downloader module.
//!
//! This module implements the Sophon chunk-based download system used by
//! HoYoverse games. It handles downloading, assembling, and updating game files
//! using a manifest-based approach with zstd-compressed chunks.

pub mod api_scrape;
pub mod game_installer;
pub mod proto_parse;
use dashmap::DashMap;
use game_installer::{DownloadHandle, UpdateInfo, read_installed_tag};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    let content = match fs::read_to_string(&path) {
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
        Err(e) => {
            log::warn!("Download state file corrupted, removing: {}", e);
            let _ = fs::remove_file(&path);
            None
        }
    }
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
    /// Installing plugins/SDKs into the game directory.
    InstallingPlugins {
        current_plugin: String,
        total_plugins: usize,
    },
    /// Downloading a plugin/SDK ZIP package.
    DownloadingPlugin {
        name: String,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    /// Download completed successfully.
    Finished,
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
    Arc::new(move |chunks: &DashMap<String, u64>| {
        let chunks_map: HashMap<String, u64> = chunks
            .iter()
            .map(|entry| (entry.key().clone(), *entry.value()))
            .collect();
        let s = DownloadState {
            game_id: meta.game_id.clone(),
            vo_lang: meta.vo_lang.clone(),
            output_path: meta.output_path.clone(),
            download_type: meta.download_type.clone(),
            current_tag: meta.current_tag.clone(),
            manifest_hash: meta.manifest_hash.clone(),
            downloaded_chunks: chunks_map,
        };
        save_download_state(&app, &s).ok();
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

    emit(&app_handle, SophonProgress::FetchingManifest);

    let (installers, tag, manifest_hash) =
        game_installer::build_installers(&client.0, &game_id, &vo_lang)
            .await
            .map_err(|e| e.to_string())?;

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
            }
            if let Err(e) = game_installer::install_channel_sdks(&client.0, &game_dir, &game_id, {
                let u = plugin_updater.clone();
                move |p| u(p)
            })
            .await
            {
                log::warn!("Channel SDK installation failed: {}", e);
            }
            emit(&app_handle, SophonProgress::Finished);
            Ok(())
        }
        Err(game_installer::SophonError::Cancelled) => Ok(()),
        Err(e) => Err(e.to_string()),
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

    emit(&app_handle, SophonProgress::FetchingManifest);

    let (installers, deleted_files, new_tag, manifest_hash) =
        game_installer::build_update_installers(&client.0, &game_id, &vo_lang, &current_tag)
            .await
            .map_err(|e| e.to_string())?;

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
            }
            if let Err(e) = game_installer::install_channel_sdks(&client.0, &game_dir, &game_id, {
                let u = plugin_updater.clone();
                move |p| u(p)
            })
            .await
            {
                log::warn!("Channel SDK installation failed: {}", e);
            }
            emit(&app_handle, SophonProgress::Finished);
            Ok(())
        }
        Err(game_installer::SophonError::Cancelled) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Pre-downloads an upcoming game version.
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

    emit(&app_handle, SophonProgress::FetchingManifest);

    let (installers, tag, manifest_hash) =
        game_installer::build_preinstall_installers(&client.0, &game_id, &vo_lang)
            .await
            .map_err(|e| e.to_string())?;

    let current_tag = game_installer::read_installed_tag(&game_dir);

    let state = DownloadState {
        game_id: game_id.clone(),
        vo_lang: vo_lang.clone(),
        output_path: output_path.clone(),
        download_type: DownloadType::Preinstall,
        current_tag,
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
            is_preinstall: true,
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
            emit(&app_handle, SophonProgress::Finished);
            Ok(())
        }
        Err(game_installer::SophonError::Cancelled) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Applies a pre-downloaded game version.
#[command]
pub async fn sophon_apply_preinstall(
    preinstall_tag: String,
    output_path: String,
    app_handle: AppHandle,
) -> Result<(), String> {
    let game_dir = app_handle
        .path()
        .resolve(&output_path, BaseDirectory::AppData)
        .map_err(|e| e.to_string())?;

    game_installer::apply_preinstall(&game_dir, &preinstall_tag)
        .await
        .map_err(|e| e.to_string())
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

    let (installers, deleted_files, tag, manifest_hash, is_preinstall) = match state.download_type {
        DownloadType::Fresh => {
            let (installers, tag, new_manifest_hash) =
                game_installer::build_installers(&client.0, &state.game_id, &state.vo_lang)
                    .await
                    .map_err(|e| e.to_string())?;
            (installers, vec![], tag, new_manifest_hash, false)
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
                .map_err(|e| e.to_string())?;
            (installers, deleted_files, tag, new_manifest_hash, false)
        }
        DownloadType::Preinstall => {
            // Guard against mixed-version corruption: if the installed version
            // changed since preinstall started, resuming would write v2.0 files
            // on top of a different base version, creating an inconsistent state.
            if let Some(ref saved_tag) = current_tag {
                let actual_tag = game_installer::read_installed_tag(&game_dir);
                if actual_tag.as_deref() != Some(saved_tag) {
                    return Err("Cannot resume preinstall: installed game version changed since preinstall started. Delete preinstall data and start over.".to_string());
                }
            }
            let (installers, tag, new_manifest_hash) = game_installer::build_preinstall_installers(
                &client.0,
                &state.game_id,
                &state.vo_lang,
            )
            .await
            .map_err(|e| e.to_string())?;
            (installers, vec![], tag, new_manifest_hash, true)
        }
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
            is_preinstall,
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
            if !is_preinstall {
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
                }
                if let Err(e) =
                    game_installer::install_channel_sdks(&client.0, &game_dir, &game_id, {
                        let u = plugin_updater.clone();
                        move |p| u(p)
                    })
                    .await
                {
                    log::warn!("Channel SDK installation failed: {}", e);
                }
            }
            emit(&app_handle, SophonProgress::Finished);
            Ok(())
        }
        Err(game_installer::SophonError::Cancelled) => Ok(()),
        Err(e) => Err(e.to_string()),
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
pub async fn sophon_pause(active: State<'_, ActiveDownload>) -> Result<(), ()> {
    if let Some(h) = active.0.lock().await.as_ref() {
        h.pause();
    }
    Ok(())
}

/// Resumes a paused download.
#[command]
pub async fn sophon_resume(active: State<'_, ActiveDownload>) -> Result<(), ()> {
    if let Some(h) = active.0.lock().await.as_ref() {
        h.resume();
    }
    Ok(())
}

/// Cancels the active download.
#[command]
pub async fn sophon_cancel(active: State<'_, ActiveDownload>) -> Result<(), ()> {
    if let Some(h) = active.0.lock().await.as_ref() {
        h.cancel();
    }
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

    game_installer::check_update(&client.0, &game_id, &vo_lang, &game_dir)
        .await
        .map_err(|e| e.to_string())
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
    game_installer::verify_integrity(&client.0, &game_id, &vo_lang, &game_dir, move |p| {
        emit(&app_clone, p)
    })
    .await
    .map_err(|e| e.to_string())
}

fn emit(app: &AppHandle, progress: SophonProgress) {
    if let Err(e) = app.emit("sophon://progress", progress) {
        log::error!("Failed to emit progress event: {}", e);
    }
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
}
