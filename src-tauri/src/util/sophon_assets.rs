use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontDoorResponse {
    pub retcode: i32,
    pub message: String,
    pub data: FrontDoorResponseData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontDoorResponseData {
    pub game_branches: Vec<GameBranch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameBranch {
    pub game: Game,
    pub main: PackageBranch,
    pub pre_download: Option<PackageBranch>,
    pub enable_base_pkg_predownload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub biz: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageBranch {
    pub package_id: String,
    pub branch: String,
    pub password: String,
    pub tag: String,
    pub diff_tags: Vec<String>,
    pub categories: Vec<Category>,
    pub required_client_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub category_id: String,
    pub matching_field: String,
    #[serde(rename = "type")]
    pub category_type: CategoryType,
    pub scenarios: Vec<CategoryScenario>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CategoryType {
    #[serde(rename = "CATEGORY_TYPE_RESOURCE")]
    Resource,
    #[serde(rename = "CATEGORY_TYPE_AUDIO")]
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CategoryScenario {
    #[serde(rename = "CATEGORY_SCENARIO_FULL")]
    Full,
    #[serde(other)]
    Unknown,
}

/// Top-level API response envelope (same shape as the getBranches endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestResposne {
    pub retcode: i32,
    pub message: String,
    pub data: BuildData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildData {
    pub build_id: String,
    pub tag: String,
    pub manifests: Vec<Manifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub category_id: String,
    pub category_name: String,
    pub matching_field: String,
    pub manifest: ManifestInfo,
    pub chunk_download: DownloadInfo,
    pub manifest_download: DownloadInfo,
    pub stats: Stats,
    pub deduplicated_stats: Stats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestInfo {
    pub id: String,
    pub checksum: String,
    pub compressed_size: String,
    pub uncompressed_size: String,
}

/// Download configuration for either chunk or manifest files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadInfo {
    /// 0 = no encryption.
    pub encryption: i32,
    pub password: String,
    /// 0 = none, 1 = compressed.
    pub compression: i32,
    pub url_prefix: String,
    pub url_suffix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub compressed_size: String,
    pub uncompressed_size: String,
    pub file_count: String,
    pub chunk_count: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SophonChunk {
    pub(crate) filename: String,
    pub(crate) size: u64,
    pub(crate) md5: String,
    pub(crate) chunks: Vec<SophonChunkData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SophonChunkData {
    pub(crate) cdn_url: String,
    pub(crate) compressed_md5: String,
    pub(crate) offset: u64,
    pub(crate) compressed_size: u64,
    pub(crate) uncompressed_size: u64,
    pub(crate) xxhash64: String,
    pub(crate) uncompressed_md5: String,
}
