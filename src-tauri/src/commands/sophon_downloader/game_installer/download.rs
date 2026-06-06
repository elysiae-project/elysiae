use std::path::Path;

use futures_util::StreamExt;
use md5::{Digest, Md5};
use reqwest::Client;
use sysinfo::Disks;
use tokio::io::AsyncWriteExt;

use super::error::{SophonError, SophonResult};
use crate::commands::sophon_downloader::api_scrape::DownloadInfo;
use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetChunk;

fn get_available_space(path: &Path) -> Option<u64> {
    let disks = Disks::new_with_refreshed_list();
    for disk in disks.iter() {
        if path.starts_with(disk.mount_point()) {
            return Some(disk.available_space());
        }
    }
    None
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

const MAX_DOWNLOAD_RETRIES: u32 = 4;

pub async fn download_chunk(
    client: &Client,
    chunk_download: &DownloadInfo,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
) -> SophonResult<()> {
    let url = chunk_download.url_for(&chunk.chunk_name);
    let mut last_err = String::new();

    for attempt in 0..MAX_DOWNLOAD_RETRIES {
        if let Some(parent) = dest.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            eprintln!("Failed to create parent directory: {}", e);
        }

        match do_download_chunk(client, &url, chunk, dest).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = e.to_string();

                if let SophonError::Http(ref req_err) = e {
                    if req_err.status() == Some(reqwest::StatusCode::RANGE_NOT_SATISFIABLE) {
                        return Ok(());
                    }
                }

                match e {
                    SophonError::SizeMismatch { .. }
                    | SophonError::Md5Mismatch { .. }
                    | SophonError::PathTraversal(_)
                    | SophonError::InvalidAssetName(_)
                    | SophonError::Cancelled => {
                        return Err(e);
                    }
                    _ => {
                        if attempt < MAX_DOWNLOAD_RETRIES - 1 {
                            eprintln!(
                                "Chunk {} failed (attempt {}/{}): {}",
                                chunk.chunk_name,
                                attempt + 1,
                                MAX_DOWNLOAD_RETRIES,
                                last_err
                            );
                            let _ = tokio::fs::remove_file(dest).await;
                            tokio::time::sleep(tokio::time::Duration::from_millis(
                                100 * (1 << attempt).min(8),
                            ))
                            .await;
                        }
                    }
                }
            }
        }
    }

    Err(SophonError::DownloadFailed {
        chunk: chunk.chunk_name.clone(),
        attempts: MAX_DOWNLOAD_RETRIES,
        error: last_err,
    })
}

async fn do_download_chunk(
    client: &Client,
    url: &str,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
) -> SophonResult<()> {
    let resp = client.get(url).send().await?;

    if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        download_full_file(client, url, chunk, dest).await
    } else {
        let resp = resp.error_for_status()?;
        download_full_file_with_response(resp, chunk, dest).await
    }
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

    let mut file = tokio::fs::File::create(dest).await?;
    let mut stream = resp.bytes_stream();
    let mut hasher = Md5::new();
    let mut total_len = 0u64;

    while let Some(chunk_bytes) = stream.next().await {
        let bytes = chunk_bytes?;
        hasher.update(&bytes);
        file.write_all(&bytes).await?;
        total_len += bytes.len() as u64;
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

async fn download_full_file(
    client: &Client,
    url: &str,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
) -> SophonResult<()> {
    let resp = client.get(url).send().await?.error_for_status()?;

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

    let mut file = tokio::fs::File::create(dest).await?;
    let mut stream = resp.bytes_stream();
    let mut hasher = Md5::new();
    let mut total_len = 0u64;

    while let Some(chunk_bytes) = stream.next().await {
        let bytes = chunk_bytes?;
        hasher.update(&bytes);
        file.write_all(&bytes).await?;
        total_len += bytes.len() as u64;
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
    async fn download_chunk_416_range_not_satisfiable() {
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
        assert!(
            result.is_ok(),
            "416 should return Ok(()) as file is already complete, got: {:?}",
            result
        );
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
}
