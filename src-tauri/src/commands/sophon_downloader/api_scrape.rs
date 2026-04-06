use serde::{Deserialize, Serialize};

// "Front Door" Structs
#[allow(unused)]
#[derive(Debug, Clone, Deserialize)]
pub struct FrontDoorResponse {
    pub retcode: i32,
    pub message: String,
    pub data: FrontDoorData,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FrontDoorData {
    pub game_branches: Vec<GameBranch>,
}

#[allow(unused)]
#[derive(Debug, Clone, Deserialize)]
pub struct GameBranch {
    pub game: GameId,
    pub main: PackageBranch,
    pub pre_download: Option<PackageBranch>,
}

#[allow(unused)]
#[derive(Debug, Clone, Deserialize)]
pub struct GameId {
    pub id: String,
    pub biz: String,
}

#[allow(unused)]
#[derive(Debug, Clone, Deserialize)]
pub struct PackageBranch {
    pub package_id: String,
    pub branch: String,
    pub password: String,
    pub tag: String,
}

// "Manifest endpoint" structs
#[allow(unused)]
#[derive(Debug, Clone, Deserialize)]
pub struct SophonBuildResponse {
    pub retcode: i32,
    pub message: String,
    pub data: SophonBuildData,
}

#[allow(unused)]
#[derive(Debug, Clone, Deserialize)]
pub struct SophonBuildData {
    pub build_id: String,
    pub tag: String,
    pub manifests: Vec<SophonManifestMeta>,
}

#[allow(unused)]
#[derive(Debug, Clone, Deserialize)]
pub struct SophonManifestMeta {
    pub category_id: String,
    pub category_name: String,
    pub matching_field: String,
    pub manifest: ManifestFileInfo,
    pub chunk_download: DownloadInfo,
    pub manifest_download: DownloadInfo,
    pub stats: Stats,
}

#[allow(unused)]
#[derive(Debug, Clone, Deserialize)]
pub struct ManifestFileInfo {
    pub id: String,
    pub checksum: String,
    pub compressed_size: String,
    pub uncompressed_size: String,
}

/// How to build a URL for either chunks or the manifest file.
///
/// URL formula  (mirrors the an-anime-team implementation):
///   `{url_prefix}{url_suffix}/{item_name}`
#[allow(unused)]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DownloadInfo {
    /// 0 = not encrypted.
    pub encryption: i32,
    pub password: String,
    /// 0 = uncompressed, 1 = zstd-compressed.
    pub compression: i32,
    pub url_prefix: String,
    pub url_suffix: String,
}

impl DownloadInfo {
    /// Build a full download URL for a named item (chunk or manifest).
    pub fn url_for(&self, item_name: &str) -> String {
        format!("{}{}/{}", self.url_prefix, self.url_suffix, item_name)
    }

    pub fn is_compressed(&self) -> bool {
        self.compression == 1
    }
}

#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub struct Stats {
    pub compressed_size: String,
    pub uncompressed_size: String,
    pub file_count: String,
    pub chunk_count: String,
}

pub fn front_door_game_index(game_id: &str) -> Option<usize> {
    match game_id.to_lowercase().as_str() {
        "bh3" => Some(3),
        "hk4e" => Some(2),
        "hkrpg" => Some(1),
        "napo" => Some(0),
        _ => None,
    }
}

pub fn vo_manifest_index(game_id: &str, vo_lang: &str) -> Option<usize> {
    if game_id.to_lowercase().contains("bh3") {
        return Some(1);
    }

    match vo_lang.to_lowercase().as_str() {
        "cn" => Some(1),
        "en" => Some(2),
        "jp" => Some(3),
        "kr" => Some(4),
        _ => None,
    }
}
