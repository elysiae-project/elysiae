use std::path::Path;

use futures_util::StreamExt;
use libc;
use md5::{Digest, Md5};
use reqwest::Client;
use tauri_plugin_log::log;
use tokio::io::{AsyncWriteExt, BufWriter};

use super::CHUNK_WRITE_BUFFER_SIZE;
use super::error::{SophonError, SophonResult};
use super::handle::DownloadHandle;
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
                    "Unknown hash format (length={len}) for verification",
                    len = expected_hash.len()
                );
                return Ok(false);
            }
        }?;
        Ok(actual == expected_hash)
    })
    .await?
}

/// Download a single chunk.
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
            match tokio::fs::OpenOptions::new().write(true).open(dest).await {
                Ok(f) => {
                    if let Err(err) = f.set_len(chunk.chunk_size).await {
                        log::warn!(
                            "Failed to truncate {name} to {size}: {err}; deleting and re-downloading",
                            name = chunk.chunk_name,
                            size = chunk.chunk_size
                        );
                        let _ = tokio::fs::remove_file(dest).await;
                        existing_size = 0;
                    } else {
                        existing_size = chunk.chunk_size;
                    }
                }
                Err(err) => {
                    log::warn!(
                        "Failed to open {name} for truncation: {err}; deleting and re-downloading",
                        name = chunk.chunk_name
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
                    "Chunk {name} has no compressed or decompressed MD5; trusting size match",
                    name = chunk.chunk_name
                );
                return Ok(());
            } else {
                log::warn!(
                    "Chunk {name} has no compressed MD5; re-downloading for integrity",
                    name = chunk.chunk_name
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
                return download_with_resume(resp, chunk, dest, existing_size, handle).await;
            }
            let _ = tokio::fs::remove_file(dest).await;
        } else {
            let _ = tokio::fs::remove_file(dest).await;
        }
    }

    // Fresh download
    let resp = client.get(url).send().await?;
    let resp = resp.error_for_status()?;
    download_full_file_with_response(resp, chunk, dest, handle).await
}

/// Download a full file, streaming body chunks to disk with MD5/XXH64 hashing.
async fn download_full_file_with_response(
    resp: reqwest::Response,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
    handle: Option<&DownloadHandle>,
) -> SophonResult<()> {
    let content_length = resp.content_length();
    if let Some(len) = content_length
        && len != chunk.chunk_size
    {
        log::warn!(
            "Content-Length ({len}) != expected chunk_size ({expected}) for {name}, proceeding anyway",
            expected = chunk.chunk_size,
            name = chunk.chunk_name
        );
    }

    check_available_space(dest, chunk.chunk_size)?;

    let file = tokio::fs::File::create(dest).await?;
    let mut file = BufWriter::with_capacity(CHUNK_WRITE_BUFFER_SIZE, file);
    let mut stream = resp.bytes_stream();
    let mut hasher = Md5::new();
    let mut xxh64_hasher: Option<xxhash_rust::xxh64::Xxh64> =
        if chunk.chunk_compressed_hash_md5.len() == 16 {
            Some(xxhash_rust::xxh64::Xxh64::new(0))
        } else {
            None
        };
    let mut total_len = 0u64;

    loop {
        let next_chunk = stream.next();
        let result = if let Some(handle) = handle {
            tokio::select! {
                biased;
                _ = handle.cancelled_future() => {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(SophonError::Cancelled);
                }
                result = next_chunk => result,
            }
        } else {
            next_chunk.await
        };

        match result {
            Some(Ok(bytes)) => {
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
                if let Some(ref mut h) = xxh64_hasher {
                    h.update(&bytes);
                }
                if let Err(err) = file.write_all(&bytes).await {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(SophonError::Io(err));
                }
            }
            Some(Err(e)) => {
                let _ = tokio::fs::remove_file(dest).await;
                return Err(e.into());
            }
            None => break,
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
                let actual = if let Some(h) = xxh64_hasher {
                    format!("{:016x}", h.digest())
                } else {
                    drop(file);
                    if verify_existing_file_hash(dest, expected).await? {
                        String::new()
                    } else {
                        expected.clone()
                    }
                };
                if !actual.is_empty() && actual != expected.to_ascii_lowercase() {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(SophonError::Md5Mismatch {
                        item: chunk.chunk_name.clone(),
                        expected: expected.clone(),
                        actual,
                    });
                }
            }
            _ => {
                log::warn!(
                    "Unknown compressed hash format (length={len}) for chunk {name}",
                    len = expected.len(),
                    name = chunk.chunk_name
                );
            }
        }
    } else {
        log::warn!(
            "Chunk {name} downloaded without compressed hash verification",
            name = chunk.chunk_name
        );
    }

    Ok(())
}

async fn download_with_resume(
    resp: reqwest::Response,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
    existing_size: u64,
    handle: Option<&DownloadHandle>,
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
    let mut xxh64_hasher: Option<xxhash_rust::xxh64::Xxh64> =
        if chunk.chunk_compressed_hash_md5.len() == 16 {
            let mut h = xxhash_rust::xxh64::Xxh64::new(0);
            {
                let existing_file = std::fs::File::open(dest)?;
                let file_len = existing_file.metadata()?.len();
                if file_len > 0 {
                    let mmap = unsafe { memmap2::Mmap::map(&existing_file)? };
                    h.update(&mmap[..existing_size as usize]);
                }
            }
            Some(h)
        } else {
            None
        };
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
    let mut file = BufWriter::with_capacity(CHUNK_WRITE_BUFFER_SIZE, file);
    let mut stream = resp.bytes_stream();
    let mut total_len = existing_size;

    loop {
        let next_chunk = stream.next();
        let result = if let Some(handle) = handle {
            tokio::select! {
                biased;
                _ = handle.cancelled_future() => {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(SophonError::Cancelled);
                }
                result = next_chunk => result,
            }
        } else {
            next_chunk.await
        };

        match result {
            Some(Ok(bytes)) => {
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
                if let Some(ref mut h) = xxh64_hasher {
                    h.update(&bytes);
                }
            }
            Some(Err(e)) => {
                let _ = tokio::fs::remove_file(dest).await;
                return Err(e.into());
            }
            None => break,
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
                let actual = if let Some(h) = xxh64_hasher {
                    format!("{:016x}", h.digest())
                } else {
                    drop(file);
                    if verify_existing_file_hash(dest, expected).await? {
                        String::new()
                    } else {
                        expected.clone()
                    }
                };
                if !actual.is_empty() && actual != expected.to_ascii_lowercase() {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(SophonError::Md5Mismatch {
                        item: chunk.chunk_name.clone(),
                        expected: expected.clone(),
                        actual,
                    });
                }
            }
            _ => {
                log::warn!(
                    "Unknown compressed hash format (length={len}) for chunk {name}",
                    len = expected.len(),
                    name = chunk.chunk_name
                );
            }
        }
    } else {
        log::warn!(
            "Chunk {name} downloaded without compressed hash verification",
            name = chunk.chunk_name
        );
    }

    Ok(())
}
