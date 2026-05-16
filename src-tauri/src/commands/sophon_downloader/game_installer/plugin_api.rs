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

    let sdks: Vec<ChannelSdkData> = resp
        .data
        .game_channel_sdks
        .into_iter()
        .filter(|sdk| {
            let filename = sdk
                .channel_sdk_pkg
                .url
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_lowercase();
            !filename.contains("dxsetup")
        })
        .collect();
    Ok(sdks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_id_for_code_known() {
        assert_eq!(game_id_for_code("hk4e"), Some("gopR6Cufr3"));
    }

    #[test]
    fn game_id_for_code_unknown() {
        assert_eq!(game_id_for_code("unknown"), None);
    }

    #[test]
    fn parse_plugin_api_response() {
        let json = r#"{
            "retcode": 0,
            "message": "OK",
            "data": {
                "plugin_releases": [{
                    "game": {"id": "gopR6Cufr3", "biz": "hk4e_global"},
                    "plugins": [
                        {
                            "plugin_id": "p1",
                            "release_id": "r1",
                            "version": "1.0",
                            "plugin_pkg": {
                                "url": "https://example.com/gme.zip",
                                "md5": "abc123",
                                "size": "1024",
                                "decompressed_size": "2048",
                                "command": null,
                                "validation": "[{\"path\":\"gme.dll\",\"md5\":\"d41d8cd98f00b204e9800998ecf8427e\",\"size\":\"100\"}]",
                                "pkg_version_file_name": null
                            }
                        },
                        {
                            "plugin_id": "p2",
                            "release_id": "r2",
                            "version": "2.0",
                            "plugin_pkg": {
                                "url": "https://example.com/other.zip",
                                "md5": "def456",
                                "size": "512",
                                "decompressed_size": "1024",
                                "command": null,
                                "validation": "[]",
                                "pkg_version_file_name": null
                            }
                        }
                    ]
                }]
            }
        }"#;
        let resp: PluginApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.plugin_releases.len(), 1);
        let plugins = &resp.data.plugin_releases[0].plugins;
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].plugin_id, "p1");
        assert_eq!(plugins[0].plugin_pkg.size, 1024);
        assert_eq!(plugins[0].plugin_pkg.validation.len(), 1);
        assert_eq!(plugins[0].plugin_pkg.validation[0].path, "gme.dll");
        assert_eq!(plugins[1].plugin_pkg.validation.len(), 0);
    }

    #[test]
    fn parse_validation_json_string() {
        let json = r#"{
            "url": "https://example.com/pkg.zip",
            "md5": "abc",
            "size": "100",
            "decompressed_size": "200",
            "command": null,
            "validation": "[{\"path\":\"file.dll\",\"md5\":\"d41d8cd98f00b204e9800998ecf8427e\",\"size\":\"50\"}]",
            "pkg_version_file_name": null
        }"#;
        let pkg: PackageData = serde_json::from_str(json).unwrap();
        assert_eq!(pkg.validation.len(), 1);
        assert_eq!(pkg.validation[0].path, "file.dll");
        assert_eq!(pkg.validation[0].size, Some(50));
    }

    #[test]
    fn parse_validation_empty_string() {
        let json = r#"{
            "url": "https://example.com/pkg.zip",
            "md5": "abc",
            "size": "100",
            "decompressed_size": "200",
            "command": null,
            "validation": "",
            "pkg_version_file_name": null
        }"#;
        let pkg: PackageData = serde_json::from_str(json).unwrap();
        assert!(pkg.validation.is_empty());
    }

    #[test]
    fn parse_validation_empty_array_string() {
        let json = r#"{
            "url": "https://example.com/pkg.zip",
            "md5": "abc",
            "size": "100",
            "decompressed_size": "200",
            "command": null,
            "validation": "[]",
            "pkg_version_file_name": null
        }"#;
        let pkg: PackageData = serde_json::from_str(json).unwrap();
        assert!(pkg.validation.is_empty());
    }

    #[test]
    fn parse_str_to_u64_valid() {
        let json = r#"{
            "url": "https://example.com/pkg.zip",
            "md5": "abc",
            "size": "12345",
            "decompressed_size": "200",
            "command": null,
            "validation": "[]",
            "pkg_version_file_name": null
        }"#;
        let pkg: PackageData = serde_json::from_str(json).unwrap();
        assert_eq!(pkg.size, 12345);
    }

    #[test]
    fn parse_optional_str_to_u64_some() {
        let json = r#"{"path":"f.dll","md5":"x","size":"999"}"#;
        let entry: ValidationEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.size, Some(999));
    }

    #[test]
    fn parse_optional_str_to_u64_empty() {
        let json = r#"{"path":"f.dll","md5":"x","size":""}"#;
        let entry: ValidationEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.size, None);
    }
}
