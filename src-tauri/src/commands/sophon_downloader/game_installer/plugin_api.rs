use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use super::error::SophonResult;

const LAUNCHER_ID: &str = "VYTpXlbWo8";
const PLUGIN_URL: &str = concat!(
    "https://sg-hyp-api.hoyoverse.com",
    "/hyp/hyp-connect/api/getGamePlugins"
);
const SDK_URL: &str = concat!(
    "https://sg-hyp-api.hoyoverse.com",
    "/hyp/hyp-connect/api/getGameChannelSDKs"
);

const GAME_IDS: &[(&str, &str)] = &[
    ("bh3", "bxPTXSET5t"),
    ("hk4e", "gopR6Cufr3"),
    ("hkrpg", "4ziysqXOQ8"),
    ("nap", "U5hbdsT9W7"),
];

pub fn game_id_for_code(code: &str) -> Option<&'static str> {
    GAME_IDS.iter().find(|(c, _)| c == &code).map(|(_, id)| *id)
}

#[derive(Debug, Clone, Deserialize)]
struct PluginApiResponse {
    #[allow(dead_code)]
    retcode: i32,
    #[allow(dead_code)]
    message: String,
    data: PluginApiData,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct PluginApiData {
    plugin_releases: Vec<PluginRelease>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginRelease {
    #[allow(dead_code)]
    pub game: GameRef,
    pub plugins: Vec<PluginPackageInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GameRef {
    #[allow(dead_code)]
    pub id: String,
    #[allow(dead_code)]
    pub biz: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginPackageInfo {
    pub plugin_id: String,
    #[allow(dead_code)]
    pub release_id: String,
    pub version: String,
    pub plugin_pkg: PackageData,
}

#[derive(Debug, Clone, Deserialize)]
struct SdkApiResponse {
    #[allow(dead_code)]
    retcode: i32,
    #[allow(dead_code)]
    message: String,
    data: SdkApiData,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct SdkApiData {
    game_channel_sdks: Vec<ChannelSdkData>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelSdkData {
    #[allow(dead_code)]
    pub game: GameRef,
    pub channel_sdk_pkg: PackageData,
    #[allow(dead_code)]
    pub pkg_version_file_name: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageData {
    pub url: String,
    pub md5: String,
    #[serde(deserialize_with = "deserialize_str_to_u64")]
    #[allow(dead_code)]
    pub size: u64,
    #[serde(deserialize_with = "deserialize_str_to_u64")]
    #[allow(dead_code)]
    pub decompressed_size: u64,
    #[allow(dead_code)]
    pub command: Option<String>,
    #[serde(deserialize_with = "deserialize_validation")]
    pub validation: Vec<ValidationEntry>,
    #[allow(dead_code)]
    pub pkg_version_file_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ValidationEntry {
    pub path: String,
    #[allow(dead_code)]
    pub md5: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_str_to_u64")]
    pub size: Option<u64>,
}

fn deserialize_str_to_u64<'de, D>(de: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = String::deserialize(de)?;
    s.parse().map_err(serde::de::Error::custom)
}

fn deserialize_optional_str_to_u64<'de, D>(de: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(de)?;
    match opt {
        Some(s) if !s.is_empty() => s.parse().map(Some).map_err(serde::de::Error::custom),
        _ => Ok(None),
    }
}

fn deserialize_validation<'de, D>(de: D) -> Result<Vec<ValidationEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: String = String::deserialize(de)?;
    if raw.is_empty() || raw == "[]" {
        return Ok(vec![]);
    }
    serde_json::from_str(&raw).map_err(serde::de::Error::custom)
}

pub async fn fetch_plugins(client: &Client, game_id: &str) -> SophonResult<Vec<PluginPackageInfo>> {
    let url = format!(
        "{}?launcher_id={}&game_ids[]={}&language=en",
        PLUGIN_URL, LAUNCHER_ID, game_id
    );

    let resp: PluginApiResponse = client
        .get(&url)
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .json()
        .await?;

    let mut plugins: Vec<PluginPackageInfo> = resp
        .data
        .plugin_releases
        .into_iter()
        .flat_map(|r| r.plugins)
        .filter(|p| {
            let filename = p
                .plugin_pkg
                .url
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_lowercase();
            !filename.contains("dxsetup")
        })
        .collect();

    plugins.dedup_by_key(|p| p.plugin_id.clone());
    Ok(plugins)
}

pub async fn fetch_channel_sdks(
    client: &Client,
    game_id: &str,
) -> SophonResult<Vec<ChannelSdkData>> {
    let url = format!(
        "{}?launcher_id={}&game_ids[]={}&language=en",
        SDK_URL, LAUNCHER_ID, game_id
    );

    let resp: SdkApiResponse = client
        .get(&url)
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .json()
        .await?;

    Ok(resp.data.game_channel_sdks)
}
