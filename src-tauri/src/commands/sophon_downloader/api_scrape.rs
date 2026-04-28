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
    pub compression: Compression,
    pub url_prefix: String,
    pub url_suffix: String,
}

/// Compression format for downloaded content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[repr(i32)]
#[serde(try_from = "i32", into = "i32")]
pub enum Compression {
    None = 0,
    Zstd = 1,
}

impl From<Compression> for i32 {
    fn from(value: Compression) -> Self {
        value as i32
    }
}

impl TryFrom<i32> for Compression {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Compression::None),
            1 => Ok(Compression::Zstd),
            _ => Err(format!("Invalid compression value: {value}")),
        }
    }
}

impl DownloadInfo {
    pub fn url_for(&self, item_name: &str) -> String {
        format!("{}{}/{}", self.url_prefix, self.url_suffix, item_name)
    }

    pub fn is_compressed(&self) -> bool {
        matches!(self.compression, Compression::Zstd)
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

#[inline]
pub fn front_door_game_index(game_id: &str) -> Option<usize> {
    match game_id.to_lowercase().as_str() {
        "bh3" => Some(3),
        "hk4e" => Some(2),
        "hkrpg" => Some(1),
        "nap" => Some(0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_door_game_index_known() {
        assert_eq!(front_door_game_index("nap"), Some(0));
        assert_eq!(front_door_game_index("hkrpg"), Some(1));
        assert_eq!(front_door_game_index("hk4e"), Some(2));
        assert_eq!(front_door_game_index("bh3"), Some(3));
    }

    #[test]
    fn front_door_game_index_unknown() {
        assert_eq!(front_door_game_index("unknown"), None);
    }

    #[test]
    fn compression_try_from_valid() {
        assert_eq!(Compression::try_from(0).unwrap(), Compression::None);
        assert_eq!(Compression::try_from(1).unwrap(), Compression::Zstd);
    }

    #[test]
    fn compression_try_from_invalid() {
        assert!(Compression::try_from(5).is_err());
    }

    #[test]
    fn download_info_url_for() {
        let dl = DownloadInfo {
            encryption: 0,
            password: String::new(),
            compression: Compression::None,
            url_prefix: "https://example.com/".to_string(),
            url_suffix: "v1".to_string(),
        };
        assert_eq!(
            dl.url_for("manifest.dat"),
            "https://example.com/v1/manifest.dat"
        );
    }

    #[test]
    fn download_info_is_compressed() {
        let zstd = DownloadInfo {
            encryption: 0,
            password: String::new(),
            compression: Compression::Zstd,
            url_prefix: String::new(),
            url_suffix: String::new(),
        };
        assert!(zstd.is_compressed());

        let none = DownloadInfo {
            encryption: 0,
            password: String::new(),
            compression: Compression::None,
            url_prefix: String::new(),
            url_suffix: String::new(),
        };
        assert!(!none.is_compressed());
    }
}
