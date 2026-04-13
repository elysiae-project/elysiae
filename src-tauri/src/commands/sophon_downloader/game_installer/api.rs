use reqwest::Client;

use crate::commands::sophon_downloader::api_scrape::{
    FrontDoorResponse, GameBranch, PackageBranch, SophonBuildData, SophonBuildResponse,
    front_door_game_index,
};
use crate::commands::sophon_downloader::proto_parse::{SophonManifestProto, decode_manifest};

use super::constants::{FRONT_DOOR_URL, SOPHON_BUILD_URL_BASE};
use super::manifest::DownloadInfo;

pub async fn fetch_front_door(
    client: &Client,
    game_id: &str,
) -> Result<(GameBranch, Option<PackageBranch>), Box<dyn std::error::Error + Send + Sync>> {
    let resp: FrontDoorResponse = client.get(FRONT_DOOR_URL).send().await?.json().await?;

    let idx =
        front_door_game_index(game_id).ok_or_else(|| format!("Unknown game_id: {game_id}"))?;

    let branch = resp
        .data
        .game_branches
        .into_iter()
        .nth(idx)
        .ok_or("Front-door branch index out of range")?;

    let pre = branch.pre_download.clone();
    Ok((branch, pre))
}

pub async fn fetch_build(
    client: &Client,
    branch: &PackageBranch,
    tag: Option<&str>,
) -> Result<SophonBuildData, Box<dyn std::error::Error + Send + Sync>> {
    let mut url = format!(
        "{}?branch={}&package_id={}&password={}",
        SOPHON_BUILD_URL_BASE, branch.branch, branch.package_id, branch.password,
    );
    if let Some(t) = tag {
        url.push_str(&format!("&tag={t}"));
    }

    let resp: SophonBuildResponse = client.get(&url).send().await?.json().await?;
    if resp.data.manifests.is_empty() {
        return Err("No manifests returned from the API".into());
    }
    Ok(resp.data)
}

pub async fn fetch_manifest(
    client: &Client,
    dl: &DownloadInfo,
    manifest_id: &str,
) -> Result<SophonManifestProto, Box<dyn std::error::Error + Send + Sync>> {
    let url = dl.url_for(manifest_id);
    let bytes = client
        .get(&url)
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

    decode_manifest(&raw).map_err(|e| e.into())
}

fn zstd_decompress(bytes: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    use std::io::Read;
    let mut decoder = zstd::Decoder::new(bytes)?;
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

pub fn vo_lang_matches(matching_field: &str, vo_lang: &str) -> bool {
    match vo_lang.to_lowercase().as_str() {
        "cn" => matching_field.contains("zh"),
        "en" => matching_field.contains("en"),
        "jp" => matching_field.contains("ja"),
        "kr" => matching_field.contains("ko"),
        _ => false,
    }
}

pub fn parse_size(s: &str) -> u64 {
    s.parse().unwrap_or(0)
}
