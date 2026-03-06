use std::fs::File;
use std::io::Write;

use reqwest::Client;
use serde::Serialize;
use tauri::command;
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Clone)]
struct DownloadProgress {
    progress: u64,
    total: u64,
}

// The bad webkit2gtk CORS policies strike back. FUCK CORS
#[command]
pub async fn download_file(
    download_url: String,
    destination: String,
    uuid: String,
    app: AppHandle,
) -> Result<(), String> {
    let client = Client::builder().build().map_err(|e| e.to_string())?;
    let response = client
        .get(&*download_url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let total = response.content_length().unwrap_or(0);

    let mut file = File::create(&destination).map_err(|e| e.to_string())?;
    let mut downloaded_bytes: u64 = 0;
    let mut last_reported: u64 = 0;
    let report_threshold = (total / 100).max(1);
    let mut stream = response.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;

        downloaded_bytes += chunk.len() as u64;
        if downloaded_bytes - last_reported >= report_threshold {
            last_reported = downloaded_bytes;
            app.emit(
                &format!("download://progress/{}", uuid),
                DownloadProgress {
                    progress: downloaded_bytes,
                    total,
                },
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
