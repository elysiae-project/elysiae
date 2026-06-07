// Protobuf data fetched from the collapse launcher project (Hi3Helper.Sophon)
use prost::Message;

// Top level of the protobuf
#[derive(Clone, PartialEq, Message)]
pub struct SophonManifestProto {
    #[prost(message, repeated, tag = "1")]
    pub assets: Vec<SophonManifestAssetProperty>,
}

// Files (nested in top level)
#[derive(Clone, PartialEq, Message)]
pub struct SophonManifestAssetProperty {
    /// Path to the file relative to the game directory.
    #[prost(string, tag = "1")]
    pub asset_name: String,

    /// Ordered list of chunks that make up this file.
    #[prost(message, repeated, tag = "2")]
    pub asset_chunks: Vec<SophonManifestAssetChunk>,

    /// 0 = regular file, 64 = directory.
    #[prost(uint32, tag = "3")]
    pub asset_type: u32,

    /// Total uncompressed file size.
    #[prost(uint64, tag = "4")]
    pub asset_size: u64,

    /// MD5 of the fully assembled file.
    #[prost(string, tag = "5")]
    pub asset_hash_md5: String,
}

impl SophonManifestAssetProperty {
    /// Returns true if this entry represents a directory (not a data file).
    /// Per the proto definition, type 64 indicates a directory.
    #[inline]
    pub fn is_directory(&self) -> bool {
        self.asset_type == 64
    }
}

// Chunks (nested in file)
#[derive(Clone, PartialEq, Message)]
pub struct SophonManifestAssetChunk {
    /// CDN object name (used to build the download URL).
    #[prost(string, tag = "1")]
    pub chunk_name: String,

    /// MD5 of the **decompressed** chunk bytes.
    #[prost(string, tag = "2")]
    pub chunk_decompressed_hash_md5: String,

    /// Byte offset in the output file where this chunk should be written.
    /// Absent for the first chunk of a file → defaults to 0.
    #[prost(uint64, tag = "3")]
    pub chunk_on_file_offset: u64,

    /// Size of the **compressed** chunk as served by the CDN.
    #[prost(uint64, tag = "4")]
    pub chunk_size: u64,

    /// Size of the chunk after decompression.
    #[prost(uint64, tag = "5")]
    pub chunk_size_decompressed: u64,

    /// Undocumented hash field — not an xxh64 per the proto comment.
    /// Not used for verification.
    #[prost(uint64, tag = "6")]
    pub chunk_compressed_hash_xxh: u64,

    /// MD5 of the **compressed** chunk bytes as served by the CDN.
    #[prost(string, tag = "7")]
    pub chunk_compressed_hash_md5: String,
}

#[inline]
pub fn decode_manifest(buf: &[u8]) -> Result<SophonManifestProto, prost::DecodeError> {
    SophonManifestProto::decode(buf)
}

#[derive(Clone, PartialEq, Message)]
pub struct SophonPatchProto {
    #[prost(message, repeated, tag = "1")]
    pub patch_assets: Vec<SophonPatchAssetProperty>,

    #[prost(message, repeated, tag = "2")]
    pub unused_assets: Vec<SophonUnusedAssetProperty>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SophonPatchAssetProperty {
    #[prost(string, tag = "1")]
    pub asset_name: String,

    #[prost(uint64, tag = "2")]
    pub asset_size: u64,

    #[prost(string, tag = "3")]
    pub asset_hash_md5: String,

    #[prost(message, repeated, tag = "4")]
    pub asset_infos: Vec<SophonPatchAssetInfo>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SophonPatchAssetInfo {
    #[prost(string, tag = "1")]
    pub version_tag: String,

    #[prost(message, optional, tag = "2")]
    pub chunk: Option<SophonPatchAssetChunk>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SophonPatchAssetChunk {
    #[prost(string, tag = "1")]
    pub patch_name: String,

    #[prost(string, tag = "2")]
    pub version_tag: String,

    #[prost(string, tag = "3")]
    pub build_id: String,

    #[prost(int64, tag = "4")]
    pub patch_size: i64,

    #[prost(string, tag = "5")]
    pub patch_md5: String,

    #[prost(int64, tag = "6")]
    pub patch_offset: i64,

    #[prost(int64, tag = "7")]
    pub patch_length: i64,

    #[prost(string, tag = "8")]
    pub original_file_name: String,

    #[prost(int64, tag = "9")]
    pub original_file_length: i64,

    #[prost(string, tag = "10")]
    pub original_file_md5: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct SophonUnusedAssetProperty {
    #[prost(string, tag = "1")]
    pub version_tag: String,

    #[prost(message, repeated, tag = "2")]
    pub asset_infos: Vec<SophonUnusedAssetInfo>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SophonUnusedAssetInfo {
    #[prost(message, repeated, tag = "1")]
    pub assets: Vec<SophonUnusedAssetFile>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SophonUnusedAssetFile {
    #[prost(string, tag = "1")]
    pub file_name: String,

    #[prost(int64, tag = "2")]
    pub file_size: i64,

    #[prost(string, tag = "3")]
    pub file_md5: String,
}

#[inline]
pub fn decode_patch_manifest(buf: &[u8]) -> Result<SophonPatchProto, prost::DecodeError> {
    SophonPatchProto::decode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_directory_type_1_is_file() {
        let prop = SophonManifestAssetProperty {
            asset_name: String::new(),
            asset_chunks: vec![],
            asset_type: 1,
            asset_size: 0,
            asset_hash_md5: "abc".into(),
        };
        assert!(!prop.is_directory());
    }

    #[test]
    fn is_directory_type_64() {
        let prop = SophonManifestAssetProperty {
            asset_name: String::new(),
            asset_chunks: vec![],
            asset_type: 64,
            asset_size: 0,
            asset_hash_md5: "nonempty".into(),
        };
        assert!(prop.is_directory());
    }

    #[test]
    fn is_directory_type_0_with_empty_hash_is_file() {
        let prop = SophonManifestAssetProperty {
            asset_name: String::new(),
            asset_chunks: vec![],
            asset_type: 0,
            asset_size: 0,
            asset_hash_md5: String::new(),
        };
        assert!(!prop.is_directory());
    }

    #[test]
    fn is_directory_is_file() {
        let prop = SophonManifestAssetProperty {
            asset_name: String::new(),
            asset_chunks: vec![],
            asset_type: 0,
            asset_size: 0,
            asset_hash_md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
        };
        assert!(!prop.is_directory());
    }

    #[test]
    fn decode_manifest_empty_buf() {
        let result = decode_manifest(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(result.is_err());
    }

    #[test]
    fn roundtrip_encode_decode() {
        let chunk = SophonManifestAssetChunk {
            chunk_name: "chunk_001".into(),
            chunk_decompressed_hash_md5: "aabbccdd".into(),
            chunk_on_file_offset: 0,
            chunk_size: 1024,
            chunk_size_decompressed: 2048,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: "eeff0011".into(),
        };
        let original = SophonManifestProto {
            assets: vec![SophonManifestAssetProperty {
                asset_name: "GameData/Data.pak".into(),
                asset_chunks: vec![chunk],
                asset_type: 0,
                asset_size: 2048,
                asset_hash_md5: "11223344".into(),
            }],
        };

        let buf = original.encode_to_vec();
        let decoded: SophonManifestProto = decode_manifest(&buf).unwrap();
        assert_eq!(original, decoded);
    }
}
