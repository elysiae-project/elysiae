use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt;
use libc;
use md5::{Digest, Md5};
use reqwest::Client;
use tauri_plugin_log::log;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::time::timeout;

use super::bandwidth::SharedBandwidthManager;
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

/// Compute MD5 hash of a file using memory-mapped I/O for efficiency.
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

/// Compute XXH64 hash of a file using memory-mapped I/O.
fn compute_file_xxh64(path: &Path) -> SophonResult<String> {
    use xxhash_rust::xxh64::Xxh64;
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len == 0 {
        let mut hasher = Xxh64::new(0);
        hasher.update(b"");
        return Ok(format!("{:016x}", hasher.digest()));
    }
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    let mut hasher = Xxh64::new(0);
    hasher.update(&mmap[..]);
    Ok(format!("{:016x}", hasher.digest()))
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

/// Download a single chunk with optimizations from the original Sophon DLL.
pub async fn download_chunk(
    client: &Client,
    chunk_download: &DownloadInfo,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
    handle: Option<&super::handle::DownloadHandle>,
    bandwidth: Option<SharedBandwidthManager>,
) -> SophonResult<()> {
    if !super::assembly::validate_chunk_name(&chunk.chunk_name) {
        return Err(SophonError::PathTraversal(chunk.chunk_name.clone().into()));
    }

    let url = chunk_download.url_for(&chunk.chunk_name);
    do_download_chunk(client, &url, chunk, dest, handle, bandwidth).await
}

async fn do_download_chunk(
    client: &Client,
    url: &str,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
    handle: Option<&super::handle::DownloadHandle>,
    bandwidth: Option<SharedBandwidthManager>,
) -> SophonResult<()> {
    // Check for partial download to resume (skip exists() to avoid TOCTOU)
    let mut existing_size = match tokio::fs::metadata(dest).await {
        Ok(meta) => meta.len(),
        Err(_) => 0,
    };

    if existing_size >= chunk.chunk_size {
        // Truncate if oversized, then verify
        if existing_size > chunk.chunk_size {
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
            let _ = tokio::fs::remove_file(dest).await;
        } else if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let resp = resp.error_for_status()?;
            let range_header_valid = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .map(|range_str| {
                    if range_str.contains("*/") {
                        return false;
                    }
                    parse_content_range_start(range_str)
                        .map(|start| start == existing_size)
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if range_header_valid {
                return download_with_resume(resp, chunk, dest, existing_size, handle, bandwidth)
                    .await;
            }
            let _ = tokio::fs::remove_file(dest).await;
        } else {
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
    download_full_file_with_response(resp, chunk, dest, handle, bandwidth).await
}

/// Optimized download using zero-copy buffers and buffer pooling.
async fn download_full_file_with_response(
    resp: reqwest::Response,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
    handle: Option<&super::handle::DownloadHandle>,
    bandwidth: Option<SharedBandwidthManager>,
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
                    // Record bandwidth metrics
                    if let Some(ref bw) = bandwidth {
                        bw.record_download(bytes.len() as u64);
                        bw.record_write(bytes.len() as u64);
                    }
                    if let Some(handle) = handle
                        && handle.is_cancelled()
                    {
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(SophonError::Cancelled);
                    }
                }
                Err(_) => {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(chunk_bytes.unwrap_err().into());
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
    bandwidth: Option<SharedBandwidthManager>,
) -> SophonResult<()> {
    if resp.status() == reqwest::StatusCode::OK {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(SophonError::ResumeFailed {
            message: "Server returned 200 OK instead of 206 Partial Content".to_string(),
        });
    }

    let expected_total = chunk.chunk_size;
    let remaining = expected_total.saturating_sub(existing_size);

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

    // Seed the hasher with existing file content using memory-mapped I/O
    let mut hasher = Md5::new();
    {
        let existing_file = std::fs::File::open(dest)?;
        let file_len = existing_file.metadata()?.len();
        if file_len > 0 {
            let mmap = unsafe { memmap2::Mmap::map(&existing_file)? };
            hasher.update(&mmap[..existing_size as usize]);
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
                    // Record bandwidth metrics
                    if let Some(ref bw) = bandwidth {
                        bw.record_download(bytes.len() as u64);
                        bw.record_write(bytes.len() as u64);
                    }
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
