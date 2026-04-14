use std::path::Path;

use bytes::BytesMut;
use futures_util::StreamExt;
use md5::{Digest, Md5};
use reqwest::Client;
use tokio::io::AsyncWriteExt;

use super::DOWNLOAD_STREAM_BUFFER_SIZE;
use super::error::{SophonError, SophonResult};
use crate::commands::sophon_downloader::api_scrape::DownloadInfo;
use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetChunk;

pub async fn download_chunk(
    client: &Client,
    chunk_download: &DownloadInfo,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
) -> SophonResult<()> {
    let url = chunk_download.url_for(&chunk.chunk_name);
    let resp = client.get(&url).send().await?.error_for_status()?;

    if let Some(len) = resp.content_length()
        && len != chunk.chunk_size
    {
        return Err(SophonError::SizeMismatch {
            item: chunk.chunk_name.clone(),
            expected: chunk.chunk_size,
            actual: len,
        });
    }

    let mut file = tokio::fs::File::create(dest).await?;
    let mut stream = resp.bytes_stream();
    let mut hasher = Md5::new();
    let mut total_len = 0u64;

    let mut buffer = BytesMut::with_capacity(DOWNLOAD_STREAM_BUFFER_SIZE);

    while let Some(chunk_bytes) = stream.next().await {
        let bytes = chunk_bytes?;
        hasher.update(&bytes);
        buffer.extend_from_slice(&bytes);
        if buffer.len() >= DOWNLOAD_STREAM_BUFFER_SIZE {
            file.write_all(&buffer).await?;
            buffer.clear();
        }
        total_len += bytes.len() as u64;
    }

    if !buffer.is_empty() {
        file.write_all(&buffer).await?;
    }

    if total_len != chunk.chunk_size {
        return Err(SophonError::SizeMismatch {
            item: chunk.chunk_name.clone(),
            expected: chunk.chunk_size,
            actual: total_len,
        });
    }

    if !chunk.chunk_compressed_hash_md5.is_empty() {
        let actual = format!("{:x}", hasher.finalize());
        if actual != chunk.chunk_compressed_hash_md5 {
            return Err(SophonError::Md5Mismatch {
                item: chunk.chunk_name.clone(),
                expected: chunk.chunk_compressed_hash_md5.clone(),
                actual,
            });
        }
    }

    Ok(())
}
