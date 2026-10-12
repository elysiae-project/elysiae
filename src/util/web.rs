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
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
static HTTP_CLIENT: OnceLock<Result<Client, reqwest::Error>> = OnceLock::new();

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
        .read_timeout(read_timeout)
        .pool_idle_timeout(Duration::from_secs(30))
}

fn http_client() -> Result<&'static Client> {
    HTTP_CLIENT
        .get_or_init(|| client_builder(READ_TIMEOUT).build())
        .as_ref()
        .context("Could not create HTTP client")
}

fn https_url(url: &str) -> Result<Url> {
    let parsed = Url::parse(url).context("Invalid request URL")?;
    ensure!(
        parsed.scheme() == "https" && parsed.host_str().is_some(),
        "Only HTTPS URLs with a host are allowed"
    );
    Ok(parsed)
}

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
    ensure!(max_size <= MAX_DOWNLOAD_SIZE, "Download budget exceeds the global limit");
    let parsed = https_url(&url)?;
    let destination = full_path(Some(dest), base_dir)?;
    download_to(http_client()?.get(parsed), destination, on_progress, max_size).await
}

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

pub async fn fetch_data<T: DeserializeOwned>(url: &str) -> Result<T> {
    let parsed = https_url(url)?;
    fetch_json(
        http_client()?.get(parsed),
        MAX_METADATA_SIZE,
        METADATA_TIMEOUT,
    )
    .await
}

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

    fn test_client(read_timeout: Duration) -> Client {
        client_builder(read_timeout)
            .https_only(false)
            .no_proxy()
            .build()
            .unwrap()
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = full_path(
                Some(PathBuf::from(format!("web-tests-{}", Uuid::new_v4()))),
                Some(BaseDirectory::Home),
            )
            .unwrap();
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn destination(&self) -> PathBuf {
            self.0.join("payload.bin")
        }

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
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Server {
        url: Url,
        task: JoinHandle<()>,
    }

    impl Server {
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
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    #[test]
    fn finalized_payload_replaces_destination() {
        run(async {
            let directory = TestDirectory::new();
            std::fs::write(directory.destination(), b"old payload").unwrap();
            let server = Server::new(
                b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\npayload",
                false,
            )
            .await;
            let client = test_client(READ_TIMEOUT);
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
            assert_eq!((last.downloaded, last.total), (7, 7));
            assert_eq!(final_contents, b"payload");
            assert!(
                events
                    .iter()
                    .all(|(event, _)| event.download_id == last.download_id)
            );
        });
    }

    #[test]
    fn chunked_download_has_unknown_total_and_creates_parents() {
        run(async {
            let directory = TestDirectory::new();
            let destination = directory.0.join("new/parent/payload.bin");
            let server = Server::new(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\npayload\r\n0\r\n\r\n",
                false,
            )
            .await;
            let client = test_client(READ_TIMEOUT);
            let progress = Arc::new(Mutex::new(Vec::new()));
            let observed = progress.clone();
            download_to(
                client.get(server.url.clone()),
                destination.clone(),
                Some(Box::new(move |event| observed.lock().unwrap().push(event))),
                7,
            )
            .await
            .unwrap();
            assert_eq!(std::fs::read(&destination).unwrap(), b"payload");
            assert_eq!(
                std::fs::read_dir(destination.parent().unwrap())
                    .unwrap()
                    .count(),
                1
            );
            let events = progress.lock().unwrap();
            assert_eq!(events.last().unwrap().downloaded, 7);
            assert!(events.iter().all(|event| event.total == 0));
        });
    }

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

    #[test]
    fn oversized_downloads_preserve_destination_and_clean_temp() {
        run(async {
            let client = test_client(READ_TIMEOUT);
            for response in [
                "HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n",
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabcd\r\n5\r\nefghi\r\n0\r\n\r\n",
                "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabcdefghi",
            ] {
                let directory = TestDirectory::new();
                std::fs::write(directory.destination(), b"original").unwrap();
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
            }
        });
    }

    #[test]
    fn failed_finalization_preserves_destination_and_cleans_temp() {
        run(async {
            let directory = TestDirectory::new();
            std::fs::create_dir(directory.destination()).unwrap();
            let original = directory.destination().join("original");
            std::fs::write(&original, b"keep").unwrap();
            let server = Server::new(
                b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\npayload",
                false,
            )
            .await;
            let error = download_to(
                test_client(READ_TIMEOUT).get(server.url.clone()),
                directory.destination(),
                None,
                7,
            )
            .await
            .unwrap_err();
            assert!(
                error.to_string().contains("Could not finalize download"),
                "{error:#}"
            );
            assert_eq!(std::fs::read(original).unwrap(), b"keep");
            assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
        });
    }

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

    #[test]
    fn stalled_headers_and_body_timeout_without_replacing_destination() {
        run(async {
            let client = test_client(Duration::from_millis(50));
            for response in ["", "HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\npartial"] {
                let directory = TestDirectory::new();
                std::fs::write(directory.destination(), b"original").unwrap();
                let server = Server::new(response, true).await;
                let error = download_to(
                    client.get(server.url.clone()),
                    directory.destination(),
                    None,
                    20,
                )
                .await
                .unwrap_err();
                assert!(
                    error.downcast_ref::<reqwest::Error>().unwrap().is_timeout(),
                    "{error:#}"
                );
                directory.assert_contents(b"original");
            }
        });
    }

    #[test]
    fn public_apis_reject_unsupported_urls_without_writing() {
        run(async {
            let directory = TestDirectory::new();
            for url in [
                "http://127.0.0.1:1/",
                "ftp://127.0.0.1/file",
                "file:///tmp/file",
                "data:application/json,{}",
                "relative",
                "https://",
            ] {
                assert!(fetch_data::<serde_json::Value>(url).await.is_err());
                let error = download_file(url.to_owned(), directory.destination(), None, None)
                    .await
                    .unwrap_err();
                assert!(
                    !error.to_string().contains("relative path"),
                    "URL must fail before path resolution: {error:#}"
                );
            }
            assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
            assert!(https_url("https://example.invalid/metadata").is_ok());
        });
    }

    #[test]
    fn caller_cannot_raise_the_global_download_limit() {
        run(async {
            assert!(download_file_limited(
                "https://example.invalid/file".into(), "file".into(), None, None,
                MAX_DOWNLOAD_SIZE + 1,
            ).await.unwrap_err().to_string().contains("global limit"));
        });
    }

    #[test]
    fn production_client_rejects_plain_http() {
        run(async {
            let error = http_client()
                .unwrap()
                .get("http://127.0.0.1:1/")
                .send()
                .await
                .unwrap_err();
            assert!(error.is_builder(), "{error:?}");
        });
    }

    #[test]
    fn redirect_policy_rejects_http_and_unsupported_schemes() {
        run(async {
            let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            target.set_nonblocking(true).unwrap();
            let client = test_client(READ_TIMEOUT);
            for scheme in ["http", "ftp"] {
                let server = Server::new(format!("HTTP/1.1 302 Found\r\nLocation: {scheme}://{}/target\r\nContent-Length: 0\r\n\r\n", target.local_addr().unwrap()), false).await;
                let error = client.get(server.url.clone()).send().await.unwrap_err();
                assert!(error.is_redirect(), "{error:?}");
                assert!(format!("{error:?}").contains("Only HTTPS redirects"));
            }
            assert_eq!(
                target.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        });
    }

    #[test]
    fn metadata_accepts_json_at_limit_with_or_without_content_length() {
        run(async {
            let client = test_client(READ_TIMEOUT);
            for response in [
                "HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"ok\":true}",
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nB\r\n{\"ok\":true}\r\n0\r\n\r\n",
            ] {
                let server = Server::new(response, false).await;
                let value: serde_json::Value =
                    fetch_json(client.get(server.url.clone()), 11, METADATA_TIMEOUT)
                        .await
                        .unwrap();
                assert_eq!(value, serde_json::json!({"ok": true}));
            }
        });
    }

    #[test]
    fn metadata_rejects_oversize_before_json_parse() {
        run(async {
            let client = test_client(READ_TIMEOUT);
            for response in [
                "HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n",
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabcd\r\n5\r\nefghi\r\n0\r\n\r\n",
                "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabcdefghi",
            ] {
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
        });
    }

    #[test]
    fn metadata_has_a_total_deadline_including_body() {
        run(async {
            let client = test_client(Duration::from_secs(10));
            let server = Server::new(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{", true).await;
            let error = fetch_json::<serde_json::Value>(
                client.get(server.url.clone()),
                11,
                Duration::from_millis(50),
            )
            .await
            .unwrap_err();
            assert!(
                error.downcast_ref::<reqwest::Error>().unwrap().is_timeout(),
                "{error:#}"
            );
        });
    }

    #[test]
    fn status_and_invalid_json_errors_are_informative() {
        run(async {
            let client = test_client(READ_TIMEOUT);
            let directory = TestDirectory::new();
            std::fs::write(directory.destination(), b"original").unwrap();
            let response = b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n";
            let server = Server::new(response, false).await;
            let error = download_to(
                client.get(server.url.clone()),
                directory.destination(),
                None,
                8,
            )
            .await
            .unwrap_err();
            assert!(
                error.to_string().contains("503 Service Unavailable"),
                "{error:#}"
            );
            directory.assert_contents(b"original");
            let server = Server::new(response, false).await;
            let error = fetch_json::<serde_json::Value>(
                client.get(server.url.clone()),
                8,
                METADATA_TIMEOUT,
            )
            .await
            .unwrap_err();
            assert!(
                error.to_string().contains("503 Service Unavailable"),
                "{error:#}"
            );
            let server =
                Server::new(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\noops", false).await;
            let error = fetch_json::<serde_json::Value>(
                client.get(server.url.clone()),
                8,
                METADATA_TIMEOUT,
            )
            .await
            .unwrap_err();
            assert_eq!(error.to_string(), "Invalid metadata JSON");
        });
    }
}
