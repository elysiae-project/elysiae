use serde::de::{self, Unexpected};
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
    #[serde(default)]
    pub diff_tags: Vec<String>,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
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

impl<'de> Deserialize<'de> for Compression {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct CompressionVisitor;
        impl<'de> de::Visitor<'de> for CompressionVisitor {
            type Value = Compression;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("0, 1, false, or true")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(if v {
                    Compression::Zstd
                } else {
                    Compression::None
                })
            }
            fn visit_i32<E: de::Error>(self, v: i32) -> Result<Self::Value, E> {
                match v {
                    0 => Ok(Compression::None),
                    1 => Ok(Compression::Zstd),
                    _ => Err(de::Error::invalid_value(
                        Unexpected::Signed(v as i64),
                        &self,
                    )),
                }
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                match v {
                    0 => Ok(Compression::None),
                    1 => Ok(Compression::Zstd),
                    _ => Err(de::Error::invalid_value(Unexpected::Signed(v), &self)),
                }
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                match v {
                    0 => Ok(Compression::None),
                    1 => Ok(Compression::Zstd),
                    _ => Err(de::Error::invalid_value(Unexpected::Unsigned(v), &self)),
                }
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                match v {
                    "true" => Ok(Compression::Zstd),
                    "false" => Ok(Compression::None),
                    _ => Err(de::Error::invalid_value(Unexpected::Str(v), &self)),
                }
            }
        }
        deserializer.deserialize_any(CompressionVisitor)
    }
}

impl Serialize for Compression {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i32(*self as i32)
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

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct SophonPatchBuildResponse {
    pub retcode: i32,
    pub message: String,
    pub data: SophonPatchBuildData,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SophonPatchBuildData {
    pub build_id: String,
    pub patch_id: String,
    pub tag: String,
    pub manifests: Vec<SophonPatchManifestMeta>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct SophonPatchManifestMeta {
    pub category_id: String,
    pub category_name: String,
    pub matching_field: String,
    pub manifest: ManifestFileInfo,
    pub diff_download: DownloadInfo,
    pub manifest_download: DownloadInfo,
    pub stats: std::collections::HashMap<String, Stats>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn compression_try_from_boundary() {
        assert!(Compression::try_from(-1).is_err());
        assert!(Compression::try_from(2).is_err());
        assert!(Compression::try_from(i32::MAX).is_err());
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
