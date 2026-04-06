pub mod api_scrape;
pub mod game_installer;
pub mod proto_parse;

use serde::{Deserialize, Serialize};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State, command};

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
) -> Result<(), String> {
    let game_dir = app_handle
        .path()
        .resolve(&output_path, BaseDirectory::AppData)
        .map_err(|e| e.to_string())?;

    let temp_dir = app_handle
        .path()
        .resolve("sophon_temp", BaseDirectory::AppData)
        .map_err(|e| e.to_string())?;

    // Fetch manifests (game data + voice-over)
    emit(&app_handle, SophonProgress::FetchingManifest);

    let installers = game_installer::build_installers(&client.0, &game_id, &vo_lang, &temp_dir)
        .await
        .map_err(|e| e.to_string())?;

    for inst in installers {
        let app_clone = app_handle.clone();
        let game_dir_clone = game_dir.clone();

        inst.install(&game_dir_clone, move |progress| {
            emit(&app_clone, progress);
        })
        .await
        .map_err(|e| e.to_string())?;
    }

    emit(&app_handle, SophonProgress::Finished);
    Ok(())
}

fn emit(app: &AppHandle, progress: SophonProgress) {
    let _ = app.emit("sophon://progress", progress);
}
