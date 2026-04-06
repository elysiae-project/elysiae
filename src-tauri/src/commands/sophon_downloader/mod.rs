pub mod api_scrape;
pub mod proto_parse;
pub mod game_installer;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, command};
use tauri::path::BaseDirectory;

pub struct HttpClient(pub reqwest::Client);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SophonProgress {
    FetchingManifest,
    Downloading {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Assembling {
        assembled_files: u64,
        total_files: u64,
    },
    Warning { message: String },
    Error { message: String },
    Finished,
}

#[command]
pub async fn sophon_download(
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

    emit(&app_handle, SophonProgress::FetchingManifest);

    let installers = game_installer::build_installers(&client.0, &game_id, &vo_lang)
        .await
        .map_err(|e| e.to_string())?;

    let app_clone = app_handle.clone();
    game_installer::install(installers, &game_dir, move |progress| {
        emit(&app_clone, progress);
    })
    .await?;

    emit(&app_handle, SophonProgress::Finished);
    Ok(())
}

fn emit(app: &AppHandle, progress: SophonProgress) {
    let _ = app.emit("sophon://progress", progress);
}
