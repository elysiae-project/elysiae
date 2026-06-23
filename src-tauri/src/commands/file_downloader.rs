use reqwest::Client;
use serde::Serialize;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, path::BaseDirectory};
use tauri::{Manager, command};
use tokio::io::AsyncWriteExt;

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
    let client = Client::builder().build().map_err(|err| err.to_string())?;
    let full_path = app_handle
        .path()
        .resolve(&dest, BaseDirectory::AppData)
        .map_err(|err| format!("download_file: failed to resolve path for {dest}: {err}"))?;

    if let Some(parent) = full_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            format!(
                "download_file: failed to create parent dir {}: {err}",
                parent.display()
            )
        })?;
    }

    let response = client
        .get(&*url)
        .send()
        .await
        .map_err(|err| err.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("download_file: HTTP {status} for {url}"));
    }
    let total = response.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&full_path).await.map_err(|err| {
        format!(
            "download_file: failed to create {}: {err}",
            full_path.display()
        )
    })?;
    let mut downloaded_bytes: u64 = 0;
    let mut last_emitted = Instant::now() - Duration::from_millis(250);
    let throttle = Duration::from_millis(250);

    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| err.to_string())?;
        file.write_all(&chunk)
            .await
            .map_err(|err| err.to_string())?;
        downloaded_bytes += chunk.len() as u64;

        if last_emitted.elapsed() >= throttle {
            last_emitted = Instant::now();
            app_handle
                .emit(
                    &format!("download://progress/{uuid}"),
                    DownloadProgress {
                        progress: downloaded_bytes,
                        total,
                    },
                )
                .map_err(|err| err.to_string())?;
        }
    }
    file.flush().await.map_err(|err| err.to_string())?;
    app_handle
        .emit(
            &format!("download://progress/{uuid}"),
            DownloadProgress {
                progress: downloaded_bytes,
                total,
            },
        )
        .map_err(|err| err.to_string())?;

    Ok(())
}
