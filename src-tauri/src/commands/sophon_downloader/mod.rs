pub mod api_scrape;
pub mod game_installer;
pub mod proto_parse;
use game_installer::{DownloadHandle, UpdateInfo};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State, command};

pub struct HttpClient(pub reqwest::Client);
pub struct ActiveDownload(pub Mutex<Option<DownloadHandle>>);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SophonProgress {
    FetchingManifest,
    Downloading {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Paused {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Assembling {
        assembled_files: u64,
        total_files: u64,
    },
    Warning {
        message: String,
    },
    Error {
        message: String,
    },
    Finished,
}

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

    let (installers, tag) = game_installer::build_installers(&client.0, &game_id, &vo_lang)
        .await
        .map_err(|e| e.to_string())?;

    let handle = DownloadHandle::new();
    *active.0.lock().unwrap() = Some(handle.clone());

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

    *active.0.lock().unwrap() = None;
    emit(&app_handle, SophonProgress::Finished);
    Ok(())
}

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

    let current_tag = game_installer::read_installed_tag_pub(&game_dir)
        .ok_or("No installed version found — cannot update")?;

    emit(&app_handle, SophonProgress::FetchingManifest);

    let (installers, deleted_files, new_tag) =
        game_installer::build_update_installers(&client.0, &game_id, &vo_lang, &current_tag)
            .await
            .map_err(|e| e.to_string())?;

    let handle = DownloadHandle::new();
    *active.0.lock().unwrap() = Some(handle.clone());

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

    *active.0.lock().unwrap() = None;
    emit(&app_handle, SophonProgress::Finished);
    Ok(())
}

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
    *active.0.lock().unwrap() = Some(handle.clone());

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

    *active.0.lock().unwrap() = None;
    emit(&app_handle, SophonProgress::Finished);
    Ok(())
}

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

    game_installer::apply_preinstall(&game_dir, &preinstall_tag).await
}

#[command]
pub fn sophon_pause(active: State<'_, ActiveDownload>) {
    if let Some(h) = active.0.lock().unwrap().as_ref() {
        h.pause();
    }
}

#[command]
pub fn sophon_resume(active: State<'_, ActiveDownload>) {
    if let Some(h) = active.0.lock().unwrap().as_ref() {
        h.resume();
    }
}

#[command]
pub fn sophon_cancel(active: State<'_, ActiveDownload>) {
    if let Some(h) = active.0.lock().unwrap().as_ref() {
        h.cancel();
    }
}

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

fn emit(app: &AppHandle, progress: SophonProgress) {
    let _ = app.emit("sophon://progress", progress);
}
