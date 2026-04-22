//! Sophon game downloader module.
//!
//! This module implements the Sophon chunk-based download system used by HoYoverse games.
//! It handles downloading, assembling, and updating game files using a manifest-based approach
//! with zstd-compressed chunks.

pub mod api_scrape;
pub mod game_installer;
pub mod proto_parse;
use game_installer::{DownloadHandle, UpdateInfo, read_installed_tag};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
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

fn download_state_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_default()
        .join(DOWNLOAD_STATE_FILE)
}

pub fn save_download_state(app: &AppHandle, state: &DownloadState) {
    let path = download_state_path(app);
    match serde_json::to_string(state) {
        Ok(json) => {
            if let Err(e) = fs::write(&path, json) {
                log::error!("Failed to save download state: {}", e);
            }
        }
        Err(e) => {
            log::error!("Failed to serialize download state: {}", e);
        }
    }
}

pub fn compute_manifest_hash(raw_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_bytes);
    hex::encode(&hasher.finalize()[..8])
}

pub fn load_download_state(app: &AppHandle) -> Option<DownloadState> {
    let path = download_state_path(app);
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn clear_download_state(app: &AppHandle) {
    let path = download_state_path(app);
    let _ = fs::remove_file(path);
}

/// Progress events emitted during download operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SophonProgress {
    /// Manifest is being fetched from the API.
    FetchingManifest,
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
    Arc::new(move |chunks: &HashMap<String, u64>| {
        let s = DownloadState {
            game_id: meta.game_id.clone(),
            vo_lang: meta.vo_lang.clone(),
            output_path: meta.output_path.clone(),
            download_type: meta.download_type.clone(),
            current_tag: meta.current_tag.clone(),
            manifest_hash: meta.manifest_hash.clone(),
            downloaded_chunks: chunks.clone(),
        };
        save_download_state(&app, &s);
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

    let (installers, tag, manifest_hash) = game_installer::build_installers(&client.0, &game_id, &vo_lang)
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
    save_download_state(&app_handle, &state);

    let handle = DownloadHandle::new();
    *active.0.lock().await = Some(handle.clone());

let saver = make_state_saver(&app_handle, &state);
let app_clone = app_handle.clone();
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
    save_download_state(&app_handle, &state);

    let handle = DownloadHandle::new();
    *active.0.lock().await = Some(handle.clone());

    let saver = make_state_saver(&app_handle, &state);
    let app_clone = app_handle.clone();
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

    let state = DownloadState {
        game_id: game_id.clone(),
        vo_lang: vo_lang.clone(),
        output_path: output_path.clone(),
        download_type: DownloadType::Preinstall,
        current_tag: None,
        manifest_hash,
        downloaded_chunks: HashMap::new(),
    };
    save_download_state(&app_handle, &state);

    let handle = DownloadHandle::new();
    *active.0.lock().await = Some(handle.clone());

    let saver = make_state_saver(&app_handle, &state);
    let app_clone = app_handle.clone();
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
    let state = load_download_state(&app_handle)
        .ok_or("No download state found to resume")?;

    let game_dir = app_handle
        .path()
        .resolve(&state.output_path, BaseDirectory::AppData)
        .map_err(|e| e.to_string())?;

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
                game_installer::build_update_installers(&client.0, &state.game_id, &state.vo_lang, &ct)
                    .await
                    .map_err(|e| e.to_string())?;
            (installers, deleted_files, tag, new_manifest_hash, false)
        }
        DownloadType::Preinstall => {
            let (installers, tag, new_manifest_hash) =
                game_installer::build_preinstall_installers(&client.0, &state.game_id, &state.vo_lang)
                    .await
                    .map_err(|e| e.to_string())?;
            (installers, vec![], tag, new_manifest_hash, true)
        }
    };

    let manifest_changed = old_manifest_hash != manifest_hash;
    let resumed_chunks = if manifest_changed {
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

/// Verifies the integrity of installed game files and re-downloads any corrupted ones.
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
  game_installer::verify_integrity(
    &client.0,
    &game_id,
    &vo_lang,
    &game_dir,
    move |p| emit(&app_clone, p),
  )
  .await
  .map_err(|e| e.to_string())
}

fn emit(app: &AppHandle, progress: SophonProgress) {
    if let Err(e) = app.emit("sophon://progress", progress) {
        log::error!("Failed to emit progress event: {}", e);
    }
}
