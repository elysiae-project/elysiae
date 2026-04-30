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
        let actual = hex::encode(hasher.finalize());
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
