use std::io::{Read, Write};
use std::time::Duration;

use reqwest::Client;

use crate::commands::sophon_downloader::api_scrape::{
    FrontDoorResponse, GameBranch, PackageBranch, SophonBuildData, SophonBuildResponse,
    SophonPatchBuildData, SophonPatchBuildResponse, SophonPatchManifestMeta,
};
use crate::commands::sophon_downloader::proto_parse::{
    SophonManifestProto, SophonPatchProto, decode_manifest, decode_patch_manifest,
};

use super::error::{SophonError, SophonResult};
use super::{FRONT_DOOR_URL, SOPHON_BUILD_URL_BASE};
use crate::commands::sophon_downloader::api_scrape::DownloadInfo;
use crate::commands::sophon_downloader::compute_content_manifest_hash;

pub struct ManifestWithHash {
    pub manifest: SophonManifestProto,
    pub hash: String,
}

const API_MAX_RETRIES: u32 = 3;

async fn fetch_json_with_retry<T: serde::de::DeserializeOwned>(
    client: &Client,
    url: &str,
    timeout_secs: u64,
) -> SophonResult<T> {
    for attempt in 0..API_MAX_RETRIES {
        let result =
            tokio::time::timeout(Duration::from_secs(timeout_secs), client.get(url).send()).await;

        match result {
            Ok(Ok(resp)) => {
                let resp = resp.error_for_status()?;
                return resp.json().await.map_err(|e| e.into());
            }
            Ok(Err(e)) => {
                if attempt == API_MAX_RETRIES - 1 {
                    return Err(e.into());
                }
            }
            Err(_) => {
                if attempt == API_MAX_RETRIES - 1 {
                    return Err(SophonError::Timeout(timeout_secs));
                }
            }
        }

        if attempt < API_MAX_RETRIES - 1 {
            tokio::time::sleep(Duration::from_secs(2u64.saturating_pow(attempt))).await;
        }
    }

    Err(SophonError::ApiError(
        -1,
        format!("Failed to fetch {} after {} retries", url, API_MAX_RETRIES),
    ))
}

pub async fn fetch_front_door(
    client: &Client,
    game_id: &str,
) -> SophonResult<(GameBranch, Option<PackageBranch>)> {
    let resp: FrontDoorResponse = fetch_json_with_retry(client, FRONT_DOOR_URL, 35).await?;

    let branch = resp
        .data
        .game_branches
        .into_iter()
        .find(|b| b.game.biz.starts_with(&format!("{}_", game_id)))
        .ok_or_else(|| SophonError::UnknownGameId(game_id.into()))?;

    let pre = branch.pre_download.clone();
    Ok((branch, pre))
}

pub async fn fetch_manifest(
    client: &Client,
    dl: &DownloadInfo,
    manifest_id: &str,
) -> SophonResult<ManifestWithHash> {
    let url = dl.url_for(manifest_id);

    let resp = tokio::time::timeout(Duration::from_secs(30), client.get(&url).send())
        .await
        .map_err(|_| SophonError::Timeout(30))??
        .error_for_status()?;

    let bytes = tokio::time::timeout(Duration::from_secs(300), resp.bytes())
        .await
        .map_err(|_| SophonError::Timeout(300))??;

    let raw = if dl.is_compressed() {
        tokio::task::spawn_blocking(move || {
            let tmp = tempfile::NamedTempFile::new()?;
            {
                let mut f = tmp.as_file();
                f.write_all(&bytes)?;
                f.flush()?;
            }
            let raw = decompress_zstd_from_file(tmp.path())?;
            Ok::<Vec<u8>, SophonError>(raw)
        })
        .await??
    } else {
        bytes.to_vec()
    };

    let manifest: SophonManifestProto =
        decode_manifest(&raw).map_err(SophonError::ManifestDecode)?;
    let hash = compute_content_manifest_hash(&manifest);
    Ok(ManifestWithHash { manifest, hash })
}

/// Decompresses zstd data from a file, keeping only the decompressed output in
/// memory.
fn decompress_zstd_from_file(path: &std::path::Path) -> SophonResult<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut decoder = zstd::Decoder::new(file)?;
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

pub async fn fetch_build(
    client: &Client,
    branch: &PackageBranch,
    tag: Option<&str>,
) -> SophonResult<SophonBuildData> {
    let tag_str = tag.unwrap_or(&branch.tag);
    let url = format!(
        "{}?branch={}&package_id={}&password={}&tag={}",
        SOPHON_BUILD_URL_BASE, branch.branch, branch.package_id, branch.password, tag_str,
    );

    let resp: SophonBuildResponse = fetch_json_with_retry(client, &url, 35).await?;
    if resp.data.manifests.is_empty() {
        return Err(SophonError::NoManifests);
    }
    Ok(resp.data)
}

pub const SOPHON_PATCH_BUILD_URL_BASE: &str = concat!(
    "https://sg-public-api.hoyoverse.com",
    "/downloader/sophon_chunk/api/getPatchBuild"
);

pub async fn fetch_patch_build(
    client: &Client,
    branch: &PackageBranch,
) -> SophonResult<SophonPatchBuildData> {
    let url = format!(
        "{}?branch={}&package_id={}&password={}&tag={}",
        SOPHON_PATCH_BUILD_URL_BASE, branch.branch, branch.package_id, branch.password, branch.tag,
    );

    let raw_resp =
        tokio::time::timeout(Duration::from_secs(35), client.post(&url).send()).await??;
    let resp: SophonPatchBuildResponse = raw_resp.error_for_status()?.json().await?;
    if resp.data.manifests.is_empty() {
        return Err(SophonError::NoManifests);
    }
    Ok(resp.data)
}

pub struct PatchManifestWithMeta {
    pub patch_manifest: SophonPatchProto,
    pub diff_download: DownloadInfo,
    pub matching_field: String,
}

pub async fn fetch_patch_manifest(
    client: &Client,
    meta: &SophonPatchManifestMeta,
) -> SophonResult<PatchManifestWithMeta> {
    let url = meta.manifest_download.url_for(&meta.manifest.id);

    let resp = tokio::time::timeout(Duration::from_secs(30), client.get(&url).send())
        .await
        .map_err(|_| SophonError::Timeout(30))??
        .error_for_status()?;

    let bytes = tokio::time::timeout(Duration::from_secs(300), resp.bytes())
        .await
        .map_err(|_| SophonError::Timeout(300))??;

    let raw = if meta.manifest_download.is_compressed() {
        tokio::task::spawn_blocking(move || {
            let tmp = tempfile::NamedTempFile::new()?;
            {
                let mut f = tmp.as_file();
                f.write_all(&bytes)?;
                f.flush()?;
            }
            let raw = decompress_zstd_from_file(tmp.path())?;
            Ok::<Vec<u8>, SophonError>(raw)
        })
        .await??
    } else {
        bytes.to_vec()
    };

    let patch_manifest =
        decode_patch_manifest(&raw).map_err(|e| SophonError::PatchManifestDecode(e.to_string()))?;

    Ok(PatchManifestWithMeta {
        patch_manifest,
        diff_download: meta.diff_download.clone(),
        matching_field: meta.matching_field.clone(),
    })
}

#[inline]
pub fn vo_lang_matches(matching_field: &str, vo_lang: &str) -> bool {
    let vo = vo_lang.as_bytes();
    let lower = matching_field.to_ascii_lowercase();
    if vo.eq_ignore_ascii_case(b"cn") {
        lower.contains("zh")
    } else if vo.eq_ignore_ascii_case(b"en") {
        lower.contains("en")
    } else if vo.eq_ignore_ascii_case(b"jp") {
        lower.contains("ja")
    } else if vo.eq_ignore_ascii_case(b"kr") {
        lower.contains("ko")
    } else {
        false
    }
}

#[inline]
pub fn is_known_vo_locale(matching_field: &str) -> bool {
    let f = matching_field.as_bytes();
    f.windows(5).any(|w| w.eq_ignore_ascii_case(b"en-us"))
        || f.windows(5).any(|w| w.eq_ignore_ascii_case(b"zh-cn"))
        || f.windows(5).any(|w| w.eq_ignore_ascii_case(b"zh-tw"))
        || f.windows(5).any(|w| w.eq_ignore_ascii_case(b"ko-kr"))
        || f.windows(5).any(|w| w.eq_ignore_ascii_case(b"ja-jp"))
}

#[inline]
pub fn parse_size(s: &str) -> SophonResult<u64> {
    s.parse()
        .map_err(|_| SophonError::InvalidSizeString(s.to_string()))
}
