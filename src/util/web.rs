use crate::core::fs::{BaseDirectory, full_path, rename};
use anyhow::{Context, Result, ensure};
use reqwest::Client;
use serde::de::DeserializeOwned;
use std::{
    path::PathBuf,
    sync::OnceLock,
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;
use url::Url;
use uuid::Uuid;
static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();
fn http_client() -> Result<Client> {
    Ok(HTTP_CLIENT
        .get_or_init(|| {
            Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(60))
                .pool_idle_timeout(Duration::from_secs(30))
                .build()
                .expect("Could not create HTTP client")
        })
        .clone())
}
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub download_id: Uuid,
    pub downloaded: u64,
    pub total: u64,
}
struct FileDownload {
    path: PathBuf,
}
impl Drop for FileDownload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
const MAX_DOWNLOAD_SIZE: u64 = 10 * 1024 * 1024 * 1024;
pub async fn download_file(
    url: String,
    dest: PathBuf,
    base_dir: Option<BaseDirectory>,
    on_progress: Option<Box<dyn Fn(DownloadProgress) + Send + 'static>>,
) -> Result<()> {
    let parsed = Url::parse(&url)?;
    ensure!(
        parsed.scheme() == "https" && parsed.host_str().is_some(),
        "Only HTTPS URLs with a host are allowed"
    );
    let res = http_client()?.get(parsed).send().await?;
    ensure!(
        res.status().is_success(),
        "HTTP request failed: {}",
        res.status()
    );
    let size = res.content_length().unwrap_or(0);
    ensure!(size <= MAX_DOWNLOAD_SIZE, "Download exceeds size limit");
    let id = Uuid::new_v4();
    let destination = full_path(Some(dest.clone()), base_dir)?;
    let destination_parent = destination
        .parent()
        .context("Download destination has no parent")?;
    tokio::fs::create_dir_all(destination_parent).await?;
    let tmp = PathBuf::from(format!(
        ".{}.download-{id}",
        destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    ));
    let path = destination_parent.join(&tmp);
    let _guard = FileDownload { path: path.clone() };
    let mut file = tokio::fs::File::create(&path).await?;
    let mut downloaded: u64 = 0;
    let mut last = Instant::now() - Duration::from_millis(250);
    let mut stream = res.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let c = chunk?;
        downloaded = downloaded
            .checked_add(c.len() as u64)
            .context("Download size overflow")?;
        ensure!(
            downloaded <= MAX_DOWNLOAD_SIZE,
            "Download exceeds size limit"
        );
        file.write_all(&c).await?;
        if last.elapsed() >= Duration::from_millis(250) {
            last = Instant::now();
            if let Some(cb) = &on_progress {
                cb(DownloadProgress {
                    download_id: id,
                    downloaded,
                    total: size,
                });
            }
        }
    }
    file.flush().await?;
    if let Some(cb) = &on_progress {
        cb(DownloadProgress {
            download_id: id,
            downloaded,
            total: size,
        });
    }
    drop(file);
    rename(crate::core::fs::MultiPathOptions {
        init_path: tmp,
        init_path_base_dir: Some(BaseDirectory::Home),
        dest_path: destination,
        dest_path_base_dir: Some(BaseDirectory::Home),
        overwrite: Some(true),
    })?;
    Ok(())
}
pub async fn fetch_data<T: DeserializeOwned>(url: &str) -> Result<T> {
    let parsed = Url::parse(url)?;
    ensure!(
        parsed.scheme() == "https" && parsed.host_str().is_some(),
        "Only HTTPS URLs with a host are allowed"
    );
    let res = http_client()?.get(parsed).send().await?;
    ensure!(
        res.status().is_success(),
        "HTTP request failed: {}",
        res.status()
    );
    Ok(res.json().await?)
}
