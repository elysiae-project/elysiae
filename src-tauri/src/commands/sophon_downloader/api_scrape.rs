use serde::{Deserialize, Serialize};

#[allow(dead_code)]
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

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct GameBranch {
    pub game: GameId,
    pub main: PackageBranch,
    pub pre_download: Option<PackageBranch>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct GameId {
    pub id: String,
    pub biz: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageBranch {
    pub package_id: String,
    pub branch: String,
    pub password: String,
    pub tag: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct SophonBuildResponse {
    pub retcode: i32,
    pub message: String,
    pub data: SophonBuildData,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct SophonBuildData {
    pub build_id: String,
    pub tag: String,
    pub manifests: Vec<SophonManifestMeta>,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ManifestFileInfo {
    pub id: String,
    pub checksum: String,
    pub compressed_size: String,
    pub uncompressed_size: String,
}

/// Describes where to download chunks or the manifest file.
///
/// URL formula: `{url_prefix}{url_suffix}/{item_name}`
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DownloadInfo {
    pub encryption: i32,
    pub password: String,
    /// 0 = uncompressed, 1 = zstd-compressed.
    pub compression: i32,
    pub url_prefix: String,
    pub url_suffix: String,
}

impl DownloadInfo {
    pub fn url_for(&self, item_name: &str) -> String {
        format!("{}{}/{}", self.url_prefix, self.url_suffix, item_name)
    }

    pub fn is_compressed(&self) -> bool {
        self.compression == 1
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
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
        "nap" => Some(0),
        _ => None,
    }
}
