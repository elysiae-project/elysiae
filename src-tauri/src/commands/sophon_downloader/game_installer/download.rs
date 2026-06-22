use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt;
use libc;
use md5::{Digest, Md5};
use reqwest::Client;
use tauri_plugin_log::log;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::time::timeout;

use super::error::{SophonError, SophonResult};
use super::{FILE_WRITE_BUFFER_SIZE, STREAM_POLL_INTERVAL_MS};
use crate::commands::sophon_downloader::api_scrape::DownloadInfo;
use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetChunk;

fn get_available_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::statvfs(cpath.as_ptr(), &mut stat) };
    if ret != 0 {
        return None;
    }
    Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

pub fn check_available_space(dest: &Path, needed: u64) -> Result<(), SophonError> {
    if let Some(available) = get_available_space(dest)
        && available < needed
    {
        return Err(SophonError::NoSpaceAvailable {
            path: dest.display().to_string(),
            needed,
            available,
        });
    }
    Ok(())
}

/// Parse Content-Range header to extract start position.
/// Returns Some(start) for "bytes START-END/TOTAL", None for unparseable.
/// Example: "bytes 500-999/1000" -> Some(500)
fn parse_content_range_start(range_str: &str) -> Option<u64> {
    let prefix = "bytes ";
    if !range_str.starts_with(prefix) {
        return None;
    }
    let after_prefix = &range_str[prefix.len()..];
    let dash_pos = after_prefix.find('-')?;
    let start_str = &after_prefix[..dash_pos];
    start_str.parse().ok()
}
fn compute_file_md5(path: &Path) -> SophonResult<String> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len == 0 {
        let mut hasher = Md5::new();
        hasher.update(b"");
        return Ok(hex::encode(hasher.finalize()));
    }
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    let mut hasher = Md5::new();
    hasher.update(&mmap[..]);
    Ok(hex::encode(hasher.finalize()))
}

fn compute_file_xxh64(path: &Path) -> SophonResult<String> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len == 0 {
        let mut hasher = twox_hash::XxHash64::default();
        std::hash::Hasher::write(&mut hasher, b"");
        return Ok(format!("{:016x}", std::hash::Hasher::finish(&hasher)));
    }
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    let mut hasher = twox_hash::XxHash64::default();
    std::hash::Hasher::write(&mut hasher, &mmap[..]);
    Ok(format!("{:016x}", std::hash::Hasher::finish(&hasher)))
}

async fn verify_existing_file_hash(path: &Path, expected_hash: &str) -> SophonResult<bool> {
    if expected_hash.is_empty() {
        return Ok(true);
    }
    let path = path.to_path_buf();
    let expected_hash = expected_hash.to_ascii_lowercase();
    tokio::task::spawn_blocking(move || {
        let actual = match expected_hash.len() {
            32 => compute_file_md5(&path),
            16 => compute_file_xxh64(&path),
            _ => {
                log::warn!(
                    "Unknown hash format (length={}) for verification",
                    expected_hash.len()
                );
                return Ok(false);
            }
        }?;
        Ok(actual == expected_hash)
    })
    .await?
}

pub async fn download_chunk(
    client: &Client,
    chunk_download: &DownloadInfo,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
    handle: Option<&super::handle::DownloadHandle>,
) -> SophonResult<()> {
    if !super::assembly::validate_chunk_name(&chunk.chunk_name) {
        return Err(SophonError::PathTraversal(chunk.chunk_name.clone().into()));
    }

    let url = chunk_download.url_for(&chunk.chunk_name);
    do_download_chunk(client, &url, chunk, dest, handle).await
}

async fn do_download_chunk(
    client: &Client,
    url: &str,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
    handle: Option<&super::handle::DownloadHandle>,
) -> SophonResult<()> {
    // Check for partial download to resume (skip exists() to avoid TOCTOU)
    let mut existing_size = match tokio::fs::metadata(dest).await {
        Ok(meta) => meta.len(),
        Err(_) => 0,
    };

    if existing_size >= chunk.chunk_size {
        // Truncate if oversized, then verify
        if existing_size > chunk.chunk_size {
            // Truncate to expected size: avoids re-downloading when extra
            // bytes were appended from a previous interrupted write.
            match tokio::fs::OpenOptions::new().write(true).open(dest).await {
                Ok(f) => {
                    if let Err(err) = f.set_len(chunk.chunk_size).await {
                        log::warn!(
                            "Failed to truncate {} to {}: {}; deleting and re-downloading",
                            chunk.chunk_name,
                            chunk.chunk_size,
                            err
                        );
                        let _ = tokio::fs::remove_file(dest).await;
                        existing_size = 0;
                    } else {
                        existing_size = chunk.chunk_size;
                    }
                }
                Err(err) => {
                    log::warn!(
                        "Failed to open {} for truncation: {}; deleting and re-downloading",
                        chunk.chunk_name,
                        err
                    );
                    let _ = tokio::fs::remove_file(dest).await;
                    existing_size = 0;
                }
            }
        }

        if existing_size >= chunk.chunk_size {
            if !chunk.chunk_compressed_hash_md5.is_empty() {
                if verify_existing_file_hash(dest, &chunk.chunk_compressed_hash_md5).await? {
                    return Ok(());
                }
                let _ = tokio::fs::remove_file(dest).await;
            } else if chunk.chunk_decompressed_hash_md5.is_empty() {
                log::warn!(
                    "Chunk {} has no compressed or decompressed MD5; trusting size match",
                    chunk.chunk_name
                );
                return Ok(());
            } else {
                log::warn!(
                    "Chunk {} has no compressed MD5; re-downloading for integrity",
                    chunk.chunk_name
                );
                let _ = tokio::fs::remove_file(dest).await;
            }
        }
    }

    if existing_size > 0 && existing_size < chunk.chunk_size {
        // Try to resume with Range request
        let range_header = format!("bytes={existing_size}-");
        let resp = client
            .get(url)
            .header(reqwest::header::RANGE, range_header)
            .timeout(Duration::from_secs(20))
            .send()
            .await?;

        if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            // 416 with incomplete file - discard partial and re-download
            let _ = tokio::fs::remove_file(dest).await;
        } else if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let resp = resp.error_for_status()?;
            // Validate Content-Range header matches what we requested
            let range_header_valid = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .map(|range_str| {
                    if range_str.contains("*/") {
                        // Server indicates resource exists but range not satisfiable
                        return false;
                    }
                    parse_content_range_start(range_str)
                        .map(|start| start == existing_size)
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if range_header_valid {
                return download_with_resume(resp, chunk, dest, existing_size, handle).await;
            }
            // Can't validate Content-Range - discard partial and re-download fresh
            let _ = tokio::fs::remove_file(dest).await;
        } else {
            // Server returned 200 OK (ignored Range header) — discard partial and
            // re-download
            let _ = tokio::fs::remove_file(dest).await;
        }
    }

    // Fresh download
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(20))
        .send()
        .await?;
    let resp = resp.error_for_status()?;
    download_full_file_with_response(resp, chunk, dest, handle).await
}

async fn download_full_file_with_response(
    resp: reqwest::Response,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
    handle: Option<&super::handle::DownloadHandle>,
) -> SophonResult<()> {
    let content_length = resp.content_length();
    if let Some(len) = content_length
        && len != chunk.chunk_size
    {
        log::warn!(
            "Content-Length ({}) != expected chunk_size ({}) for {}, proceeding anyway",
            len,
            chunk.chunk_size,
            chunk.chunk_name
        );
    }

    check_available_space(dest, chunk.chunk_size)?;

    let file = tokio::fs::File::create(dest).await?;
    let mut file = BufWriter::with_capacity(FILE_WRITE_BUFFER_SIZE, file);
    let mut stream = resp.bytes_stream();
    let mut hasher = Md5::new();
    let mut total_len = 0u64;

    loop {
        match timeout(
            Duration::from_millis(STREAM_POLL_INTERVAL_MS),
            stream.next(),
        )
        .await
        {
            Ok(Some(chunk_bytes)) => match chunk_bytes {
                Ok(bytes) => {
                    if bytes.is_empty() && total_len < chunk.chunk_size {
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(SophonError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "corrupted compressed data: empty chunk while data remaining",
                        )));
                    }
                    total_len += bytes.len() as u64;
                    if total_len > chunk.chunk_size {
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(SophonError::SizeMismatch {
                            item: chunk.chunk_name.clone(),
                            expected: chunk.chunk_size,
                            actual: total_len,
                        });
                    }
                    hasher.update(&bytes);
                    if let Err(err) = file.write_all(&bytes).await {
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(SophonError::Io(err));
                    }
                    if let Some(handle) = handle
                        && handle.is_cancelled()
                    {
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(SophonError::Cancelled);
                    }
                }
                Err(_) => {
                    // Network error reading from the response stream. The
                    // partial bytes already written are not necessarily a
                    // valid prefix, so discard to avoid spurious resume
                    // attempts over a corrupt window.
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(chunk_bytes.unwrap_err().into());
                }
            },
            Ok(None) => break,
            Err(_) => {
                // Idle poll window elapsed. Check cancellation before
                // continuing; if the user hasn't asked to stop, treat the
                // stall as a benign pause and resume polling. This keeps
                // cancel responsiveness tight (≈ STREAM_POLL_INTERVAL_MS)
                // even over a stalled TCP connection.
                if let Some(handle) = handle
                    && handle.is_cancelled()
                {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(SophonError::Cancelled);
                }
                continue;
            }
        }
    }

    if let Err(err) = file.flush().await {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(SophonError::Io(err));
    }

    if total_len != chunk.chunk_size {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(SophonError::SizeMismatch {
            item: chunk.chunk_name.clone(),
            expected: chunk.chunk_size,
            actual: total_len,
        });
    }

    if !chunk.chunk_compressed_hash_md5.is_empty() {
        let expected = &chunk.chunk_compressed_hash_md5;
        match expected.len() {
            32 => {
                let actual = hex::encode(hasher.finalize());
                if actual != expected.to_ascii_lowercase() {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(SophonError::Md5Mismatch {
                        item: chunk.chunk_name.clone(),
                        expected: expected.clone(),
                        actual,
                    });
                }
            }
            16 => {
                drop(file);
                if !verify_existing_file_hash(dest, expected).await? {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(SophonError::Md5Mismatch {
                        item: chunk.chunk_name.clone(),
                        expected: expected.clone(),
                        actual: "(xxh64 mismatch)".to_string(),
                    });
                }
            }
            _ => {
                log::warn!(
                    "Unknown compressed hash format (length={}) for chunk {}",
                    expected.len(),
                    chunk.chunk_name
                );
            }
        }
    } else {
        log::warn!(
            "Chunk {} downloaded without compressed hash verification",
            chunk.chunk_name
        );
    }

    Ok(())
}

async fn download_with_resume(
    resp: reqwest::Response,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
    existing_size: u64,
    handle: Option<&super::handle::DownloadHandle>,
) -> SophonResult<()> {
    if resp.status() == reqwest::StatusCode::OK {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(SophonError::ResumeFailed {
            message: "Server returned 200 OK instead of 206 Partial Content".to_string(),
        });
    }

    let expected_total = chunk.chunk_size;
    let remaining = expected_total.saturating_sub(existing_size);

    // Content-Length may be absent for chunked transfer-encoding.
    // If present, validate it matches the expected remaining bytes.
    if let Some(len) = resp.content_length()
        && len != remaining
    {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(SophonError::SizeMismatch {
            item: chunk.chunk_name.clone(),
            expected: remaining,
            actual: len,
        });
    }

    check_available_space(dest, remaining)?;

    // Seed the hasher with existing file content so we can do incremental hashing
    // during the append — avoids re-reading the entire file for MD5 verification.
    let mut hasher = Md5::new();
    {
        let existing_file = tokio::fs::File::open(dest).await?;
        let mut reader =
            tokio::io::BufReader::with_capacity(super::FILE_WRITE_BUFFER_SIZE, existing_file);
        let mut buf = vec![0u8; super::FILE_WRITE_BUFFER_SIZE];
        loop {
            let n = reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }

    let file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(dest)
        .await?;
    let mut file = BufWriter::with_capacity(FILE_WRITE_BUFFER_SIZE, file);
    let mut stream = resp.bytes_stream();
    let mut total_len = existing_size;

    loop {
        match timeout(
            Duration::from_millis(STREAM_POLL_INTERVAL_MS),
            stream.next(),
        )
        .await
        {
            Ok(Some(chunk_bytes_res)) => match chunk_bytes_res {
                Ok(bytes) => {
                    if bytes.is_empty() && total_len < expected_total {
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(SophonError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "corrupted compressed data: empty chunk while data remaining",
                        )));
                    }
                    total_len += bytes.len() as u64;
                    if total_len > expected_total {
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(SophonError::SizeMismatch {
                            item: chunk.chunk_name.clone(),
                            expected: expected_total,
                            actual: total_len,
                        });
                    }
                    if let Err(err) = file.write_all(&bytes).await {
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(SophonError::Io(err));
                    }
                    hasher.update(&bytes);
                    if let Some(handle) = handle
                        && handle.is_cancelled()
                    {
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(SophonError::Cancelled);
                    }
                }
                Err(_) => {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(chunk_bytes_res.unwrap_err().into());
                }
            },
            Ok(None) => break,
            Err(_) => {
                if let Some(handle) = handle
                    && handle.is_cancelled()
                {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(SophonError::Cancelled);
                }
                continue;
            }
        }
    }

    if let Err(err) = file.flush().await {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(SophonError::Io(err));
    }

    if total_len != expected_total {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(SophonError::SizeMismatch {
            item: chunk.chunk_name.clone(),
            expected: expected_total,
            actual: total_len,
        });
    }

    // Verify hash using the accumulated incremental hash (no full-file re-read for
    // MD5)
    if !chunk.chunk_compressed_hash_md5.is_empty() {
        let expected = &chunk.chunk_compressed_hash_md5;
        match expected.len() {
            32 => {
                let actual = hex::encode(hasher.finalize());
                if actual != expected.to_ascii_lowercase() {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(SophonError::Md5Mismatch {
                        item: chunk.chunk_name.clone(),
                        expected: expected.clone(),
                        actual,
                    });
                }
            }
            16 => {
                drop(file);
                if !verify_existing_file_hash(dest, expected).await? {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(SophonError::Md5Mismatch {
                        item: chunk.chunk_name.clone(),
                        expected: expected.clone(),
                        actual: "(xxh64 mismatch)".to_string(),
                    });
                }
            }
            _ => {
                log::warn!(
                    "Unknown compressed hash format (length={}) for chunk {}",
                    expected.len(),
                    chunk.chunk_name
                );
            }
        }
    } else {
        log::warn!(
            "Chunk {} downloaded without compressed hash verification",
            chunk.chunk_name
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use md5::{Digest, Md5};
    use reqwest::Client;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::error::SophonError;
    use super::check_available_space;
    use super::download_chunk;
    use super::parse_content_range_start;
    use crate::commands::sophon_downloader::api_scrape::{Compression, DownloadInfo};
    use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetChunk;

    fn make_download_info(server: &MockServer) -> DownloadInfo {
        let server_uri = server.uri();
        DownloadInfo {
            encryption: 0,
            password: String::new(),
            compression: Compression::None,
            url_prefix: format!("{server_uri}/"),
            url_suffix: "chunks".to_string(),
        }
    }

    fn make_chunk(chunk_name: &str, chunk_size: u64, md5: &str) -> SophonManifestAssetChunk {
        SophonManifestAssetChunk {
            chunk_name: chunk_name.to_string(),
            chunk_decompressed_hash_md5: String::new(),
            chunk_on_file_offset: 0,
            chunk_size,
            chunk_size_decompressed: chunk_size,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: md5.to_string(),
            chunk_old_offset: -1,
        }
    }

    fn dest_path() -> PathBuf {
        tempfile::tempdir().unwrap().keep().join("chunk.bin")
    }

    #[tokio::test]
    async fn download_chunk_success() {
        let server = MockServer::start().await;
        let data = b"hello world".to_vec();
        let expected_md5 = hex::encode(Md5::digest(&data));
        let chunk = make_chunk("test_chunk", data.len() as u64, &expected_md5);
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/test_chunk"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn download_chunk_size_mismatch_content_length() {
        let server = MockServer::start().await;
        let data = b"hello world".to_vec();
        let chunk = make_chunk("test_chunk", 9999, "irrelevant");
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/test_chunk"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, SophonError::SizeMismatch { expected: 9999, actual, .. } if actual == data.len() as u64),
            "expected SizeMismatch with expected=9999, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn download_chunk_size_mismatch_total() {
        let server = MockServer::start().await;
        let data = b"short".to_vec();
        let chunk = make_chunk("test_chunk", 1024, "");
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/test_chunk"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                SophonError::SizeMismatch {
                    expected: 1024,
                    actual: 5,
                    ..
                }
            ),
            "expected SizeMismatch with expected=1024 actual=5, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn download_chunk_md5_mismatch() {
        let server = MockServer::start().await;
        let data = b"hello world".to_vec();
        let chunk = make_chunk(
            "test_chunk",
            data.len() as u64,
            "badmd5hash0000000000000000000000",
        );
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/test_chunk"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, SophonError::Md5Mismatch { .. }),
            "expected Md5Mismatch, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn download_chunk_empty_hash_skips_md5() {
        let server = MockServer::start().await;
        let data = b"hello world".to_vec();
        let chunk = make_chunk("test_chunk", data.len() as u64, "");
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/test_chunk"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn download_chunk_http_error() {
        let server = MockServer::start().await;
        let chunk = make_chunk("test_chunk", 100, "irrelevant");
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/test_chunk"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn download_chunk_416_on_plain_get_is_error() {
        // 416 on a plain GET is non-standard and should be treated as an error
        let server = MockServer::start().await;
        let chunk = make_chunk("test_chunk", 100, "");
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/test_chunk"))
            .respond_with(ResponseTemplate::new(416).set_body_bytes("Range not satisfiable"))
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;
        assert!(result.is_err(), "416 on plain GET should be an error");
    }

    #[tokio::test]
    async fn download_chunk_writes_to_file() {
        let server = MockServer::start().await;
        let data = b"downloaded payload content".to_vec();
        let expected_md5 = hex::encode(Md5::digest(&data));
        let chunk = make_chunk("test_chunk", data.len() as u64, &expected_md5);
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/test_chunk"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();
        download_chunk(&client, &dl_info, &chunk, &dest, None)
            .await
            .unwrap();

        let written = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(written, data);
    }

    #[tokio::test]
    async fn download_chunk_large_content() {
        let server = MockServer::start().await;
        let data = vec![0xAB_u8; 256 * 1024 * 3 + 512];
        let expected_md5 = hex::encode(Md5::digest(&data));
        let chunk = make_chunk("large_chunk", data.len() as u64, &expected_md5);
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/large_chunk"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();
        download_chunk(&client, &dl_info, &chunk, &dest, None)
            .await
            .unwrap();

        let written = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(written.len(), data.len());
        assert_eq!(written, data);
    }

    /// Helper: construct a `DownloadInfo` without spinning up a `MockServer`.
    ///
    /// Used by path-validation rejection tests where validation fails before
    /// any HTTP call, so the URL prefix is irrelevant.
    fn dummy_download_info() -> DownloadInfo {
        DownloadInfo {
            encryption: 0,
            password: String::new(),
            compression: Compression::None,
            url_prefix: "http://invalid.test/".to_string(),
            url_suffix: "chunks".to_string(),
        }
    }

    /// Path-traversal validation must reject an empty chunk name before any
    /// HTTP request is constructed. The error variant per download.rs is
    /// always `SophonError::PathTraversal` for any validation failure.
    #[tokio::test]
    async fn download_chunk_rejects_empty_chunk_name() {
        // ARRANGE: chunk with empty name; DownloadInfo URL prefix is irrelevant
        // because validation short-circuits before `url_for` is called.
        let chunk = make_chunk("", 100, "irrelevant");
        let dl_info = dummy_download_info();
        let client = Client::new();
        let dest = dest_path();

        // ACT
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;

        // ASSERT: must be rejected with PathTraversal, not an HTTP/IO error.
        assert!(result.is_err(), "empty chunk_name must be rejected");
        assert!(
            matches!(result.unwrap_err(), SophonError::PathTraversal(_)),
            "expected PathTraversal, got non-matching error variant"
        );
    }

    /// A `..` path component (`foo/../bar`) is a classic traversal attempt and
    /// must be rejected before any HTTP request is constructed.
    #[tokio::test]
    async fn download_chunk_rejects_dotdot_component() {
        // ARRANGE: chunk whose name contains a parent-directory component
        let chunk = make_chunk("foo/../bar", 100, "irrelevant");
        let dl_info = dummy_download_info();
        let client = Client::new();
        let dest = dest_path();

        // ACT
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;

        // ASSERT
        assert!(result.is_err(), "'foo/../bar' must be rejected");
        assert!(matches!(result.unwrap_err(), SophonError::PathTraversal(_)));
    }

    /// An absolute POSIX path must be rejected as traversal.
    #[tokio::test]
    async fn download_chunk_rejects_absolute_path() {
        // ARRANGE: chunk whose name is a leading-slash absolute path
        let chunk = make_chunk("/etc/passwd", 100, "irrelevant");
        let dl_info = dummy_download_info();
        let client = Client::new();
        let dest = dest_path();

        // ACT
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;

        // ASSERT
        assert!(result.is_err(), "'/etc/passwd' must be rejected");
        assert!(matches!(result.unwrap_err(), SophonError::PathTraversal(_)));
    }

    /// A leading backslash must be rejected so a Windows-style absolute path
    /// cannot be smuggled in even on non-Windows hosts.
    #[tokio::test]
    async fn download_chunk_rejects_backslash_prefix() {
        // ARRANGE: chunk whose name begins with a backslash
        let chunk = make_chunk("\\Windows\\System32", 100, "irrelevant");
        let dl_info = dummy_download_info();
        let client = Client::new();
        let dest = dest_path();

        // ACT
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;

        // ASSERT
        assert!(result.is_err(), "'\\Windows\\System32' must be rejected");
        assert!(matches!(result.unwrap_err(), SophonError::PathTraversal(_)));
    }

    /// A Windows drive-letter prefix (`C:\Windows`) must be rejected; allowing
    /// it would let an attacker redirect the download to an arbitrary host
    /// path on Windows clients.
    #[tokio::test]
    async fn download_chunk_rejects_drive_letter() {
        // ARRANGE: chunk whose name starts with a Windows drive letter
        let chunk = make_chunk("C:\\Windows", 100, "irrelevant");
        let dl_info = dummy_download_info();
        let client = Client::new();
        let dest = dest_path();

        // ACT
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;

        // ASSERT
        assert!(result.is_err(), "'C:\\Windows' must be rejected");
        assert!(matches!(result.unwrap_err(), SophonError::PathTraversal(_)));
    }

    /// A null byte in the chunk name must be rejected. Some runtimes treat
    /// null as a string terminator and would happily use whatever follows as
    /// the real path on disk; rejecting here closes that hole entirely.
    #[tokio::test]
    async fn download_chunk_rejects_null_byte() {
        // ARRANGE: chunk whose name contains a null byte
        let chunk = make_chunk("evil\0chunk", 100, "irrelevant");
        let dl_info = dummy_download_info();
        let client = Client::new();
        let dest = dest_path();

        // ACT
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;

        // ASSERT
        assert!(result.is_err(), "null byte in chunk_name must be rejected");
        assert!(matches!(result.unwrap_err(), SophonError::PathTraversal(_)));
    }

    /// Regression guard: consecutive dots *inside a single filename component*
    /// (e.g. `chunk_v1.0..2.0`) are legitimate version-like names and MUST NOT
    /// be rejected as traversal. Only `..` appearing as a separator-delimited
    /// path component is treated as a traversal pattern.
    #[tokio::test]
    async fn download_chunk_allows_consecutive_dots() {
        // ARRANGE: mock server returns the expected payload at the URL path
        // derived from the allowed chunk name.
        let server = MockServer::start().await;
        let data = b"versioned chunk payload".to_vec();
        let expected_md5 = hex::encode(Md5::digest(&data));
        let chunk = make_chunk("chunk_v1.0..2.0", data.len() as u64, &expected_md5);
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/chunk_v1.0..2.0"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();

        // ACT
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;

        // ASSERT: validation succeeds AND the download writes the payload to
        // disk. If the regression had returned, validation would short-circuit
        // with PathTraversal before any HTTP traffic occurred.
        assert!(
            result.is_ok(),
            "chunk with consecutive dots inside a single component must be allowed, got: {result:?}"
        );
        let written = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(written, data);
    }

    /// Positive control: a normal alphanumeric chunk name passes validation
    /// and the download proceeds end-to-end against the mock server.
    #[tokio::test]
    async fn download_chunk_allows_normal_chunk_name() {
        // ARRANGE
        let server = MockServer::start().await;
        let data = b"plain chunk payload".to_vec();
        let expected_md5 = hex::encode(Md5::digest(&data));
        let chunk = make_chunk("abc123", data.len() as u64, &expected_md5);
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/abc123"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();

        // ACT
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;

        // ASSERT: validation accepted and download completed.
        assert!(result.is_ok(), "normal chunk name must be allowed");
        let written = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(written, data);
    }

    #[test]
    fn parse_content_range_start_standard() {
        assert_eq!(parse_content_range_start("bytes 500-999/1000"), Some(500));
    }

    #[test]
    fn parse_content_range_start_zero_start() {
        assert_eq!(parse_content_range_start("bytes 0-999/1000"), Some(0));
    }

    #[test]
    fn parse_content_range_start_no_prefix() {
        assert_eq!(parse_content_range_start("500-999/1000"), None);
    }

    #[test]
    fn parse_content_range_start_empty() {
        assert_eq!(parse_content_range_start(""), None);
    }

    #[test]
    fn parse_content_range_start_invalid() {
        assert_eq!(parse_content_range_start("bytes abc-999/1000"), None);
    }

    #[test]
    fn parse_content_range_start_large_values() {
        assert_eq!(
            parse_content_range_start(
                "bytes 18446744073709551615-18446744073709551615/18446744073709551615"
            ),
            Some(18446744073709551615)
        );
    }

    #[test]
    fn check_available_space_zero_needed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check_available_space(dir.path(), 0).is_ok());
    }

    #[test]
    fn check_available_space_small_needed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check_available_space(dir.path(), 1).is_ok());
    }

    #[test]
    fn check_available_space_ludicrous_needed() {
        let dir = tempfile::tempdir().unwrap();
        let result = check_available_space(dir.path(), u64::MAX);
        assert!(result.is_err());
        match result.unwrap_err() {
            SophonError::NoSpaceAvailable { .. } => {}
            other => panic!("expected NoSpaceAvailable, got: {other:?}"),
        }
    }

    #[test]
    fn check_available_space_nonexistent_path() {
        let bad_path = std::path::PathBuf::from("/nonexistent_path_xyzzy_42");
        assert!(check_available_space(&bad_path, 0).is_ok());
        assert!(check_available_space(&bad_path, u64::MAX).is_ok());
    }

    #[tokio::test]
    async fn download_chunk_xxh64_hash_verified() {
        use std::hash::Hasher;
        let server = MockServer::start().await;
        let data = b"hello world xxh64 test data".to_vec();
        let mut hasher = twox_hash::XxHash64::default();
        hasher.write(&data);
        let expected_xxh64 = format!("{:016x}", hasher.finish());

        let chunk = make_chunk("test_chunk", data.len() as u64, &expected_xxh64);
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/test_chunk"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;
        assert!(
            result.is_ok(),
            "expected Ok with matching XXH64, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn download_chunk_xxh64_mismatch() {
        let server = MockServer::start().await;
        let data = b"hello world xxh64 test data".to_vec();
        let chunk = make_chunk("test_chunk", data.len() as u64, "0000000000000000");
        let dl_info = make_download_info(&server);

        Mock::given(method("GET"))
            .and(path("chunks/test_chunk"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let dest = dest_path();
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, SophonError::Md5Mismatch { .. }),
            "expected Md5Mismatch for XXH64, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn download_chunk_truncates_oversized_existing_file() {
        let server = MockServer::start().await;
        let data = b"hello world".to_vec();
        let expected_md5 = hex::encode(md5::Md5::digest(data.as_slice()));

        // Pre-create a file larger than chunk_size with garbage at the end
        let dest = dest_path();
        let oversized: Vec<u8> = data.iter().chain(b"extra_garbage_bytes").cloned().collect();
        tokio::fs::write(&dest, &oversized).await.unwrap();
        let pre_size = tokio::fs::metadata(&dest).await.unwrap().len();
        assert!(pre_size > data.len() as u64);

        // Server is mounted but won't be called — the oversized file should be
        // truncated to chunk_size, then verified via MD5, and return Ok.
        Mock::given(method("GET"))
            .and(path("chunks/test_chunk"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = Client::new();
        let chunk = make_chunk("test_chunk", data.len() as u64, &expected_md5);
        let dl_info = make_download_info(&server);
        let result = download_chunk(&client, &dl_info, &chunk, &dest, None).await;
        let post_size = tokio::fs::metadata(&dest).await.unwrap().len();
        assert!(
            result.is_ok(),
            "expected Ok after truncation+verify, got: {result:?}",
        );
        assert_eq!(
            post_size,
            data.len() as u64,
            "file should be truncated to chunk_size"
        );
    }
}
