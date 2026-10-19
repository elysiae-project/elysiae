use crate::core::fs::{BaseDirectory, full_path};
use anyhow::{Context, Result, ensure};
use reqwest::{Client, ClientBuilder, RequestBuilder, Response, redirect::Policy};
use serde::de::DeserializeOwned;
use std::{
    path::PathBuf,
    sync::OnceLock,
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;
use url::Url;
use uuid::Uuid;

const MAX_DOWNLOAD_SIZE: u64 = 10 * 1024 * 1024 * 1024;
const MAX_METADATA_SIZE: usize = 4 * 1024 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(4 * 60 * 60);
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
static HTTP_CLIENT: OnceLock<Result<Client, reqwest::Error>> = OnceLock::new();

/// Configure an HTTPS-only client with bounded requests and safe redirects.
fn client_builder(read_timeout: Duration) -> ClientBuilder {
    Client::builder()
        .https_only(true)
        .redirect(Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" || attempt.url().host_str().is_none() {
                attempt.error("Only HTTPS redirects with a host are allowed")
            } else {
                Policy::default().redirect(attempt)
            }
        }))
        .connect_timeout(Duration::from_secs(15))
        .timeout(DOWNLOAD_TIMEOUT)
        .read_timeout(read_timeout)
        .pool_idle_timeout(Duration::from_secs(30))
}

/// Initialize and return the shared HTTP client.
pub(crate) fn http_client() -> Result<&'static Client> {
    HTTP_CLIENT
        .get_or_init(|| client_builder(READ_TIMEOUT).build())
        .as_ref()
        .context("Could not create HTTP client")
}

/// Parse a request URL, requiring HTTPS and a host.
fn https_url(url: &str) -> Result<Url> {
    let parsed = Url::parse(url).context("Invalid request URL")?;
    ensure!(
        parsed.scheme() == "https" && parsed.host_str().is_some(),
        "Only HTTPS URLs with a host are allowed"
    );
    Ok(parsed)
}

/// Send a request and reject unsuccessful status codes or oversized declared bodies.
async fn checked_response(request: RequestBuilder, max_size: u64, kind: &str) -> Result<Response> {
    let response = request
        .send()
        .await
        .with_context(|| format!("{kind} request failed"))?;
    ensure!(
        response.status().is_success(),
        "HTTP request failed for {}: {}",
        response.url(),
        response.status()
    );
    if let Some(size) = response.content_length() {
        ensure!(
            size <= max_size,
            "{kind} Content-Length {size} bytes exceeds limit of {max_size} bytes"
        );
    }
    Ok(response)
}

/// Download byte counts and identifier; a zero total means the size is unknown.
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub download_id: Uuid,
    pub downloaded: u64,
    pub total: u64,
}

/// Removes a download's staging file when the transfer ends or is cancelled.
struct FileDownload {
    path: PathBuf,
}

impl Drop for FileDownload {
    /// Remove any unfinished download at the guarded path.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A caller may impose a smaller resource budget, never increase the global
/// limit. Both declared and streamed sizes are checked before finalization.
pub async fn download_file_limited(
    url: String,
    dest: PathBuf,
    base_dir: Option<BaseDirectory>,
    on_progress: Option<Box<dyn Fn(DownloadProgress) + Send + 'static>>,
    max_size: u64,
) -> Result<()> {
    ensure!(
        max_size <= MAX_DOWNLOAD_SIZE,
        "Download budget exceeds the global limit"
    );
    let parsed = https_url(&url)?;
    let destination = full_path(Some(dest), base_dir)?;
    download_to(
        http_client()?.get(parsed),
        destination,
        on_progress,
        max_size,
    )
    .await
}

/// Stream a bounded response into a staging file and atomically replace the destination.
async fn download_to(
    request: RequestBuilder,
    destination: PathBuf,
    on_progress: Option<Box<dyn Fn(DownloadProgress) + Send + 'static>>,
    max_size: u64,
) -> Result<()> {
    let mut response = checked_response(request, max_size, "Download").await?;
    let size = response.content_length();
    let id = Uuid::new_v4();
    let destination_parent = destination
        .parent()
        .context("Download destination has no parent")?;
    tokio::fs::create_dir_all(destination_parent).await?;
    let path = destination_parent.join(format!(".download-{id}"));
    // Synchronous creation avoids a cancelled Tokio open creating a file after
    // the cleanup guard has already run. Only guard files we actually created.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("Could not create download file {}", path.display()))?;
    let _guard = FileDownload { path: path.clone() };
    let mut file = tokio::fs::File::from_std(file);
    let mut downloaded: u64 = 0;
    let mut last = Instant::now() - Duration::from_millis(250);
    while let Some(chunk) = response.chunk().await.with_context(|| {
        format!("Download body failed after {downloaded} bytes (declared length: {size:?})")
    })? {
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .context("Download size overflow")?;
        ensure!(
            downloaded <= max_size,
            "Download exceeds size limit of {max_size} bytes (received {downloaded} bytes)"
        );
        file.write_all(&chunk)
            .await
            .context("Could not write downloaded data")?;
        if last.elapsed() >= Duration::from_millis(250) {
            last = Instant::now();
            if let Some(cb) = &on_progress {
                cb(DownloadProgress {
                    download_id: id,
                    downloaded,
                    total: size.unwrap_or(0),
                });
            }
        }
    }
    if let Some(expected) = size {
        ensure!(
            downloaded == expected,
            "Download length mismatch: expected {expected} bytes, received {downloaded} bytes"
        );
    }
    file.flush()
        .await
        .context("Could not flush downloaded file")?;
    drop(file);
    // These are resolved, same-parent paths.
    // Commit without an await so cancellation cannot race a background rename.
    std::fs::rename(&path, &destination)
        .with_context(|| format!("Could not finalize download to {}", destination.display()))?;
    if let Some(cb) = &on_progress {
        cb(DownloadProgress {
            download_id: id,
            downloaded,
            total: size.unwrap_or(0),
        });
    }
    Ok(())
}

/// Fetch HTTPS JSON metadata within the shared size and time limits.
pub async fn fetch_data<T: DeserializeOwned>(url: &str) -> Result<T> {
    let parsed = https_url(url)?;
    fetch_json(
        http_client()?.get(parsed),
        MAX_METADATA_SIZE,
        METADATA_TIMEOUT,
    )
    .await
}

/// Read and deserialize JSON while enforcing response size and request deadlines.
async fn fetch_json<T: DeserializeOwned>(
    request: RequestBuilder,
    max_size: usize,
    timeout: Duration,
) -> Result<T> {
    let mut response =
        checked_response(request.timeout(timeout), max_size as u64, "Metadata").await?;
    let size = response.content_length();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.with_context(|| {
        format!(
            "Metadata body failed after {} bytes (declared length: {size:?})",
            bytes.len()
        )
    })? {
        ensure!(
            chunk.len() <= max_size - bytes.len(),
            "Metadata exceeds size limit of {max_size} bytes"
        );
        bytes.extend_from_slice(&chunk);
    }
    if let Some(expected) = size {
        ensure!(
            bytes.len() as u64 == expected,
            "Metadata length mismatch: expected {expected} bytes, received {} bytes",
            bytes.len()
        );
    }
    serde_json::from_slice(&bytes).context("Invalid metadata JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Allows HTTPS requests while rejecting insecure and local-file URLs.
    #[test]
    fn requests_require_https() {
        assert!(https_url("https://example.invalid/file").is_ok());
        for url in [
            "http://example.invalid/file",
            "ftp://example.invalid/file",
            "file:///tmp/file",
        ] {
            assert!(https_url(url).is_err());
        }
    }
}
