use std::time::Duration;

use reqwest::Client;

use crate::commands::sophon_downloader::api_scrape::{
    FrontDoorResponse, GameBranch, PackageBranch, SophonBuildData, SophonBuildResponse,
    front_door_game_index,
};
use crate::commands::sophon_downloader::proto_parse::{SophonManifestProto, decode_manifest};

use super::error::{SophonError, SophonResult};
use super::{FRONT_DOOR_URL, SOPHON_BUILD_URL_BASE};
use crate::commands::sophon_downloader::api_scrape::DownloadInfo;
use crate::commands::sophon_downloader::compute_content_manifest_hash;

pub struct ManifestWithHash {
    pub manifest: SophonManifestProto,
    pub hash: String,
}

pub async fn fetch_front_door(
    client: &Client,
    game_id: &str,
) -> SophonResult<(GameBranch, Option<PackageBranch>)> {
    let resp: FrontDoorResponse = client
        .get(FRONT_DOOR_URL)
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .json()
        .await?;

    let idx =
        front_door_game_index(game_id).ok_or_else(|| SophonError::UnknownGameId(game_id.into()))?;

    let branch = resp
        .data
        .game_branches
        .into_iter()
        .nth(idx)
        .ok_or(SophonError::BranchIndexOutOfRange)?;

    let pre = branch.pre_download.clone();
    Ok((branch, pre))
}

pub async fn fetch_manifest(
    client: &Client,
    dl: &DownloadInfo,
    manifest_id: &str,
) -> SophonResult<ManifestWithHash> {
    let url = dl.url_for(manifest_id);
    let bytes = client
        .get(&url)
        .timeout(Duration::from_secs(120))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let raw = if dl.is_compressed() {
        tokio::task::spawn_blocking(move || zstd_decompress(&bytes)).await??
    } else {
        bytes.to_vec()
    };

    let manifest: SophonManifestProto =
        decode_manifest(&raw).map_err(SophonError::ManifestDecode)?;
    let hash = compute_content_manifest_hash(&manifest);
    Ok(ManifestWithHash { manifest, hash })
}

pub async fn fetch_build(
    client: &Client,
    branch: &PackageBranch,
    tag: Option<&str>,
) -> SophonResult<SophonBuildData> {
    let mut url = format!(
        "{}?branch={}&package_id={}&password={}",
        SOPHON_BUILD_URL_BASE, branch.branch, branch.package_id, branch.password,
    );
    if let Some(t) = tag {
        url.push_str(&format!("&tag={t}"));
    }

    let resp: SophonBuildResponse = client
        .get(&url)
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .json()
        .await?;
    if resp.data.manifests.is_empty() {
        return Err(SophonError::NoManifests);
    }
    Ok(resp.data)
}

fn zstd_decompress(bytes: &[u8]) -> SophonResult<Vec<u8>> {
    use std::io::Read;
    let mut decoder = zstd::Decoder::new(bytes)?;
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

#[inline]
pub fn vo_lang_matches(matching_field: &str, vo_lang: &str) -> bool {
    match vo_lang.to_lowercase().as_str() {
        "cn" => matching_field.contains("zh"),
        "en" => matching_field.contains("en"),
        "jp" => matching_field.contains("ja"),
        "kr" => matching_field.contains("ko"),
        _ => false,
    }
}

#[inline]
pub fn is_known_vo_locale(matching_field: &str) -> bool {
    let lower = matching_field.to_lowercase();
    lower.contains("en-us")
        || lower.contains("zh-cn")
        || lower.contains("zh-tw")
        || lower.contains("ko-kr")
        || lower.contains("ja-jp")
}

#[inline]
pub fn parse_size(s: &str) -> u64 {
    s.parse().unwrap_or(0)
}
