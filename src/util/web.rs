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

/// Download an HTTPS file within the global size limit, reporting optional progress.
pub async fn download_file(
    url: String,
    dest: PathBuf,
    base_dir: Option<BaseDirectory>,
    on_progress: Option<Box<dyn Fn(DownloadProgress) + Send + 'static>>,
) -> Result<()> {
    download_file_limited(url, dest, base_dir, on_progress, MAX_DOWNLOAD_SIZE).await
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
    // These are resolved, same-parent paths, not inputs for core::fs::rename.
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
    use std::{
        future::Future,
        sync::{Arc, Mutex},
    };
    use tokio::{io::AsyncReadExt, net::TcpListener, task::JoinHandle};

    /// Run an async HTTP fixture test with a five-second deadline.
    fn run(test: impl Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(5), test)
                    .await
                    .expect("HTTP fixture timed out");
            });
    }

    /// Build a local fixture client with no proxy and production redirect checks.
    fn test_client(read_timeout: Duration) -> Client {
        client_builder(read_timeout)
            .https_only(false)
            .no_proxy()
            .build()
            .unwrap()
    }

    /// Isolated download directory removed when the fixture is dropped.
    struct TestDirectory(PathBuf);

    impl TestDirectory {
        /// Create an isolated directory for download fixtures.
        fn new() -> Self {
            let path = full_path(
                Some(PathBuf::from(format!("web-tests-{}", Uuid::new_v4()))),
                Some(BaseDirectory::Home),
            )
            .unwrap();
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        /// Return the fixture's download destination.
        fn destination(&self) -> PathBuf {
            self.0.join("payload.bin")
        }

        /// Check the final payload and confirm no staging files remain.
        fn assert_contents(&self, expected: &[u8]) {
            assert_eq!(std::fs::read(self.destination()).unwrap(), expected);
            assert_eq!(
                std::fs::read_dir(&self.0).unwrap().count(),
                1,
                "temporary file leaked"
            );
        }
    }

    impl Drop for TestDirectory {
        /// Remove the fixture directory and its contents.
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Single-request local HTTP server with an abortable fixture task.
    struct Server {
        url: Url,
        task: JoinHandle<()>,
    }

    impl Server {
        /// Serve a fixed response, optionally keeping it open to simulate a stall.
        async fn new(response: impl AsRef<[u8]>, hold_open: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = Url::parse(&format!(
                "http://{}/fixture",
                listener.local_addr().unwrap()
            ))
            .unwrap();
            let response = response.as_ref().to_vec();
            let task = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(socket.read_u8().await.unwrap());
                    assert!(request.len() < 8192);
                }
                socket.write_all(&response).await.unwrap();
                if hold_open {
                    std::future::pending::<()>().await;
                }
            });
            Self { url, task }
        }
    }

    impl Drop for Server {
        /// Stop the server task when the fixture is dropped.
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    /// Verify fixed-length and chunked payloads commit before reporting final progress.
    #[test]
    fn finalized_payload_replaces_destination() {
        run(async {
            let client = test_client(READ_TIMEOUT);
            for (response, total) in [
                ("HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\npayload", 7),
                (
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\npayload\r\n0\r\n\r\n",
                    0,
                ),
            ] {
                let directory = TestDirectory::new();
                std::fs::write(directory.destination(), b"old payload").unwrap();
                let server = Server::new(response, false).await;
                let progress = Arc::new(Mutex::new(Vec::new()));
                let observed = progress.clone();
                let destination = directory.destination();
                download_to(
                    client.get(server.url.clone()),
                    directory.destination(),
                    Some(Box::new(move |event| {
                        observed
                            .lock()
                            .unwrap()
                            .push((event, std::fs::read(&destination).unwrap()));
                    })),
                    7,
                )
                .await
                .unwrap();
                directory.assert_contents(b"payload");
                let events = progress.lock().unwrap();
                let (last, final_contents) = events.last().unwrap();
                assert_eq!((last.downloaded, last.total), (7, total));
                assert_eq!(final_contents, b"payload");
                assert!(events.iter().all(
                    |(event, _)| event.download_id == last.download_id && event.total == total
                ));
            }
        });
    }

    /// Verify interrupted streams retain the destination and remove staging files.
    #[test]
    fn interrupted_streams_preserve_destination_and_clean_temp() {
        run(async {
            let client = test_client(READ_TIMEOUT);
            for response in [
                "HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\nshort",
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nshort\r\n",
            ] {
                let directory = TestDirectory::new();
                std::fs::write(directory.destination(), b"original").unwrap();
                let server = Server::new(response, false).await;
                let error = download_to(
                    client.get(server.url.clone()),
                    directory.destination(),
                    None,
                    32,
                )
                .await
                .unwrap_err();
                assert!(
                    error.to_string().contains("Download body failed"),
                    "{error:#}"
                );
                directory.assert_contents(b"original");
            }
        });
    }

    /// Verify download/JSON size bounds and caller limits without replacing the destination.
    #[test]
    fn oversized_downloads_preserve_destination_and_clean_temp() {
        run(async {
            let client = test_client(READ_TIMEOUT);
            let directory = TestDirectory::new();
            std::fs::write(directory.destination(), b"original").unwrap();
            for response in [
                "HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n",
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabcd\r\n5\r\nefghi\r\n0\r\n\r\n",
            ] {
                let server = Server::new(response, false).await;
                let error = download_to(
                    client.get(server.url.clone()),
                    directory.destination(),
                    None,
                    8,
                )
                .await
                .unwrap_err();
                assert!(error.to_string().contains("limit of 8 bytes"), "{error:#}");
                directory.assert_contents(b"original");
                let server = Server::new(response, false).await;
                let error = fetch_json::<serde_json::Value>(
                    client.get(server.url.clone()),
                    8,
                    METADATA_TIMEOUT,
                )
                .await
                .unwrap_err();
                assert!(error.to_string().contains("limit of 8 bytes"), "{error:#}");
            }
            let error = download_file_limited(
                "https://127.0.0.1:1/file".into(),
                directory.destination(),
                None,
                None,
                MAX_DOWNLOAD_SIZE + 1,
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains("global limit"), "{error:#}");
            directory.assert_contents(b"original");
        });
    }

    /// Verify cancellation preserves the destination and cleans partial files.
    #[test]
    fn cancelled_download_preserves_destination_and_cleans_temp() {
        run(async {
            let directory = TestDirectory::new();
            std::fs::write(directory.destination(), b"original").unwrap();
            let server = Server::new(
                b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\npartial",
                true,
            )
            .await;
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let task = tokio::spawn(download_to(
                test_client(READ_TIMEOUT).get(server.url.clone()),
                directory.destination(),
                Some(Box::new(move |_| {
                    let _ = tx.send(());
                })),
                20,
            ));
            rx.recv().await.unwrap();
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            directory.assert_contents(b"original");
        });
    }

    /// Verify the total request deadline stops downloads even while bytes keep arriving.
    #[test]
    fn request_deadline_bounds_downloads_even_without_read_inactivity() {
        run(async {
            let directory = TestDirectory::new();
            std::fs::write(directory.destination(), b"original").unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/slow", listener.local_addr().unwrap());
            let task = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n")
                    .await
                    .unwrap();
                loop {
                    if socket.write_all(b"x").await.is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            });
            let result = download_to(
                test_client(Duration::from_secs(10))
                    .get(url)
                    .timeout(Duration::from_millis(60)),
                directory.destination(),
                None,
                1000,
            )
            .await;
            task.abort();
            assert!(
                result
                    .unwrap_err()
                    .downcast_ref::<reqwest::Error>()
                    .unwrap()
                    .is_timeout()
            );
            directory.assert_contents(b"original");
        });
    }

    /// Reject direct and redirected insecure URLs without contacting the target or writing files.
    #[test]
    fn redirect_policy_rejects_http_and_unsupported_schemes() {
        run(async {
            let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            target.set_nonblocking(true).unwrap();
            let client = test_client(READ_TIMEOUT);
            let directory = TestDirectory::new();
            std::fs::write(directory.destination(), b"original").unwrap();
            for scheme in ["http", "ftp"] {
                let url = format!("{scheme}://{}/target", target.local_addr().unwrap());
                let error = fetch_data::<serde_json::Value>(&url).await.unwrap_err();
                assert!(error.to_string().contains("Only HTTPS URLs"), "{error:#}");
                let error = download_file(url.clone(), directory.destination(), None, None)
                    .await
                    .unwrap_err();
                assert!(error.to_string().contains("Only HTTPS URLs"), "{error:#}");
                let server = Server::new(
                    format!("HTTP/1.1 302 Found\r\nLocation: {url}\r\nContent-Length: 0\r\n\r\n"),
                    false,
                )
                .await;
                let error = client.get(server.url.clone()).send().await.unwrap_err();
                assert!(error.is_redirect(), "{error:?}");
                assert!(format!("{error:?}").contains("Only HTTPS redirects"));
                directory.assert_contents(b"original");
            }
            assert_eq!(
                target.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        });
    }
}
