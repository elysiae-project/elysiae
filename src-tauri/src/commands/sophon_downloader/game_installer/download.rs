use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt;
use libc;
use md5::{Digest, Md5};
use reqwest::Client;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::time::timeout;

use super::MD5_HASH_BUFFER_SIZE;
use super::error::{SophonError, SophonResult};
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
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
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

async fn compute_file_md5(path: &Path) -> SophonResult<String> {
    let mut file = tokio::io::BufReader::new(tokio::fs::File::open(path).await?);
    let mut hasher = Md5::new();
    let mut buf = [0u8; MD5_HASH_BUFFER_SIZE];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub async fn download_chunk(
    client: &Client,
    chunk_download: &DownloadInfo,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
) -> SophonResult<()> {
    if chunk.chunk_name.is_empty() {
        return Err(SophonError::InvalidAssetName(
            "chunk_name cannot be empty".into(),
        ));
    }
    if chunk.chunk_name.contains('\0') {
        return Err(SophonError::InvalidAssetName(
            "chunk_name cannot contain null bytes".into(),
        ));
    }
    let mut chars = chunk.chunk_name.chars();
    if let (Some(first), Some(':')) = (chars.next(), chars.next())
        && first.is_ascii_alphabetic()
    {
        return Err(SophonError::PathTraversal(chunk.chunk_name.clone().into()));
    }
    if chunk.chunk_name.starts_with('/')
        || chunk.chunk_name.starts_with('\\')
        || chunk.chunk_name.contains("..")
    {
        return Err(SophonError::PathTraversal(chunk.chunk_name.clone().into()));
    }

    let url = chunk_download.url_for(&chunk.chunk_name);
    do_download_chunk(client, &url, chunk, dest).await
}

async fn do_download_chunk(
    client: &Client,
    url: &str,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
) -> SophonResult<()> {
    // Check for partial download to resume (skip exists() to avoid TOCTOU)
    let existing_size = match tokio::fs::metadata(dest).await {
        Ok(meta) => meta.len(),
        Err(_) => 0,
    };

    if existing_size >= chunk.chunk_size {
        // File is already complete (or larger than expected)
        if existing_size > chunk.chunk_size {
            let _ = tokio::fs::remove_file(dest).await;
        } else {
            // Verify MD5 of existing complete file
            if !chunk.chunk_compressed_hash_md5.is_empty() {
                let actual = compute_file_md5(dest).await?;
                if actual == chunk.chunk_compressed_hash_md5 {
                    return Ok(());
                }
                // MD5 mismatch - remove and re-download
                let _ = tokio::fs::remove_file(dest).await;
            } else {
                return Ok(());
            }
        }
    }

    if existing_size > 0 && existing_size < chunk.chunk_size {
        // Try to resume with Range request
        let range_header = format!("bytes={}-", existing_size);
        let resp = client
            .get(url)
            .header(reqwest::header::RANGE, range_header)
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
                return download_with_resume(resp, chunk, dest, existing_size).await;
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
    let resp = client.get(url).send().await?;
    let resp = resp.error_for_status()?;
    download_full_file_with_response(resp, chunk, dest).await
}

async fn download_full_file_with_response(
    resp: reqwest::Response,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
) -> SophonResult<()> {
    let len = match resp.content_length() {
        Some(l) => l,
        None => {
            return Err(SophonError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!(
                    "server did not send Content-Length for chunk '{}'",
                    chunk.chunk_name
                ),
            )));
        }
    };
    if len != chunk.chunk_size {
        return Err(SophonError::SizeMismatch {
            item: chunk.chunk_name.clone(),
            expected: chunk.chunk_size,
            actual: len,
        });
    }

    check_available_space(dest, chunk.chunk_size)?;

    let file = tokio::fs::File::create(dest).await?;
    let mut file = BufWriter::new(file);
    let mut stream = resp.bytes_stream();
    let mut hasher = Md5::new();
    let mut total_len = 0u64;

    loop {
        match timeout(Duration::from_millis(20000), stream.next()).await {
            Ok(Some(chunk_bytes)) => {
                let bytes = chunk_bytes?;
                total_len += bytes.len() as u64;
                hasher.update(&bytes);
                file.write_all(&bytes).await?;
            }
            Ok(None) => break,
            Err(_) => continue, // keep looping to allow cancellation or data arrival
        }
    }

    file.flush().await?;

    if total_len != chunk.chunk_size {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(SophonError::SizeMismatch {
            item: chunk.chunk_name.clone(),
            expected: chunk.chunk_size,
            actual: total_len,
        });
    }

    if !chunk.chunk_compressed_hash_md5.is_empty() {
        let actual = hex::encode(hasher.finalize());
        if actual != chunk.chunk_compressed_hash_md5 {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(SophonError::Md5Mismatch {
                item: chunk.chunk_name.clone(),
                expected: chunk.chunk_compressed_hash_md5.clone(),
                actual,
            });
        }
    }

    Ok(())
}

async fn download_with_resume(
    resp: reqwest::Response,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
    existing_size: u64,
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

    let file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(dest)
        .await?;
    let mut file = BufWriter::new(file);
    let mut stream = resp.bytes_stream();
    let mut total_len = existing_size;

    loop {
        match timeout(Duration::from_millis(20000), stream.next()).await {
            Ok(Some(chunk_bytes)) => {
                let bytes = chunk_bytes?;
                file.write_all(&bytes).await?;
                total_len += bytes.len() as u64;
            }
            Ok(None) => break,
            Err(_) => continue, // timeout: loop back, allows responsive cancellation
        }
    }

    file.flush().await?;

    if total_len != expected_total {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(SophonError::SizeMismatch {
            item: chunk.chunk_name.clone(),
            expected: expected_total,
            actual: total_len,
        });
    }

    // Verify MD5 of the complete file after resume
    if !chunk.chunk_compressed_hash_md5.is_empty() {
        let actual = compute_file_md5(dest).await?;
        if actual != chunk.chunk_compressed_hash_md5 {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(SophonError::Md5Mismatch {
                item: chunk.chunk_name.clone(),
                expected: chunk.chunk_compressed_hash_md5.clone(),
                actual,
            });
        }
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

    use super::super::DOWNLOAD_STREAM_BUFFER_SIZE;
    use super::super::error::SophonError;
    use super::download_chunk;
    use super::parse_content_range_start;
    use crate::commands::sophon_downloader::api_scrape::{Compression, DownloadInfo};
    use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetChunk;

    fn make_download_info(server: &MockServer) -> DownloadInfo {
        DownloadInfo {
            encryption: 0,
            password: String::new(),
            compression: Compression::None,
            url_prefix: format!("{}/", server.uri()),
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
        let result = download_chunk(&client, &dl_info, &chunk, &dest).await;
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
        let result = download_chunk(&client, &dl_info, &chunk, &dest).await;
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
        let result = download_chunk(&client, &dl_info, &chunk, &dest).await;
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
            "badmd5hash00000000000000000",
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
        let result = download_chunk(&client, &dl_info, &chunk, &dest).await;
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
        let result = download_chunk(&client, &dl_info, &chunk, &dest).await;
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
        let result = download_chunk(&client, &dl_info, &chunk, &dest).await;
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
        let result = download_chunk(&client, &dl_info, &chunk, &dest).await;
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
        download_chunk(&client, &dl_info, &chunk, &dest)
            .await
            .unwrap();

        let written = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(written, data);
    }

    #[tokio::test]
    async fn download_chunk_large_content() {
        let server = MockServer::start().await;
        let data = vec![0xAB_u8; DOWNLOAD_STREAM_BUFFER_SIZE * 3 + 512];
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
        download_chunk(&client, &dl_info, &chunk, &dest)
            .await
            .unwrap();

        let written = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(written.len(), data.len());
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
}
