use reqwest::Client;
use serde::Serialize;
use std::fs::File;
use std::io::Write;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, path::BaseDirectory};
use tauri::{Manager, command};

#[derive(Serialize, Clone)]
struct DownloadProgress {
    progress: u64,
    total: u64,
}

#[command]
pub async fn download_file(
    url: String,
    dest: String,
    uuid: String,
    app_handle: AppHandle,
) -> Result<(), String> {
    let client = Client::builder().build().map_err(|e| e.to_string())?;
    let full_path = app_handle
        .path()
        .resolve(&dest, BaseDirectory::AppData)
        .unwrap();

    let response = client.get(&*url).send().await.map_err(|e| e.to_string())?;
    let total = response.content_length().unwrap_or(0);
    let mut file = File::create(&full_path).map_err(|e| e.to_string())?;
    let mut downloaded_bytes: u64 = 0;
    let mut last_emitted = Instant::now() - Duration::from_millis(250);
    let throttle = Duration::from_millis(250);

    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        downloaded_bytes += chunk.len() as u64;

        if last_emitted.elapsed() >= throttle {
            last_emitted = Instant::now();
            app_handle
                .emit(
                    &format!("download://progress/{}", uuid),
                    DownloadProgress {
                        progress: downloaded_bytes,
                        total,
                    },
                )
                .map_err(|e| e.to_string())?;
        }
    }
    // Emit one last event after download is complete
    app_handle
        .emit(
            &format!("download://progress/{}", uuid),
            DownloadProgress {
                progress: downloaded_bytes,
                total,
            },
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}
