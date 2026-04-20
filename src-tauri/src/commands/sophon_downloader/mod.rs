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
use std::fs;
use std::path::PathBuf;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State, command};
use tauri_plugin_log::log;

/// HTTP client wrapper for dependency injection.
pub struct HttpClient(pub reqwest::Client);

/// Thread-safe container for the active download handle.
pub struct ActiveDownload(pub tokio::sync::Mutex<Option<DownloadHandle>>);

/// Persisted state for download resumption after app restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadState {
    pub game_id: String,
    pub vo_lang: String,
    pub output_path: String,
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
    if let Err(e) = fs::write(&path, serde_json::to_string(state).unwrap_or_default()) {
        log::error!("Failed to save download state: {}", e);
    }
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

  let state = DownloadState {
    game_id: game_id.clone(),
    vo_lang: vo_lang.clone(),
    output_path: output_path.clone(),
  };
  save_download_state(&app_handle, &state);

  emit(&app_handle, SophonProgress::FetchingManifest);

  let (installers, tag) = game_installer::build_installers(&client.0, &game_id, &vo_lang)
    .await
    .map_err(|e| e.to_string())?;

  let handle = DownloadHandle::new();
  *active.0.lock().await = Some(handle.clone());

  let app_clone = app_handle.clone();
  game_installer::install(
    installers,
    &game_dir,
    vec![],
    &tag,
    false,
    handle,
    move |p| emit(&app_clone, p),
  )
  .await?;

  *active.0.lock().await = None;
  clear_download_state(&app_handle);
  emit(&app_handle, SophonProgress::Finished);
  Ok(())
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

    let (installers, deleted_files, new_tag) =
        game_installer::build_update_installers(&client.0, &game_id, &vo_lang, &current_tag)
            .await
            .map_err(|e| e.to_string())?;

    let handle = DownloadHandle::new();
    *active.0.lock().await = Some(handle.clone());

    let app_clone = app_handle.clone();
    game_installer::install(
        installers,
        &game_dir,
        deleted_files,
        &new_tag,
        false,
        handle,
        move |p| emit(&app_clone, p),
    )
    .await?;

    *active.0.lock().await = None;
    emit(&app_handle, SophonProgress::Finished);
    Ok(())
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

    let (installers, tag) =
        game_installer::build_preinstall_installers(&client.0, &game_id, &vo_lang)
            .await
            .map_err(|e| e.to_string())?;

    let handle = DownloadHandle::new();
    *active.0.lock().await = Some(handle.clone());

    let app_clone = app_handle.clone();
    game_installer::install(
        installers,
        &game_dir,
        vec![],
        &tag,
        true, // is_preinstall
        handle,
        move |p| emit(&app_clone, p),
    )
    .await?;

    *active.0.lock().await = None;
    emit(&app_handle, SophonProgress::Finished);
    Ok(())
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
/// Loads the saved download state and restarts the download,
/// skipping already-valid chunks via manifest comparison.
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

  emit(&app_handle, SophonProgress::FetchingManifest);

  let (installers, tag) = game_installer::build_installers(&client.0, &state.game_id, &state.vo_lang)
    .await
    .map_err(|e| e.to_string())?;

  let handle = DownloadHandle::new();
  *active.0.lock().await = Some(handle.clone());

  let app_clone = app_handle.clone();
  game_installer::install(
    installers,
    &game_dir,
    vec![],
    &tag,
    false,
    handle,
    move |p| emit(&app_clone, p),
  )
  .await?;

  *active.0.lock().await = None;
  clear_download_state(&app_handle);
  emit(&app_handle, SophonProgress::Finished);
  Ok(())
}

/// Checks if there is a downloadable state to resume.
#[command]
pub async fn sophon_has_resume_state(app_handle: AppHandle) -> bool {
  load_download_state(&app_handle).is_some()
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
