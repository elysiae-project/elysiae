// Sophon protobuf manifest
use prost::Message;

// Top-level manifest
#[derive(Clone, PartialEq, Message)]
pub struct SophonManifestProto {
    #[prost(message, repeated, tag = "1")]
    pub assets: Vec<SophonManifestAssetProperty>,
}

// Asset properties
#[derive(Clone, PartialEq, Message)]
pub struct SophonManifestAssetProperty {
    /// Asset path relative to the game directory.
    #[prost(string, tag = "1")]
    pub asset_name: String,

    /// Chunks composing this asset.
    #[prost(message, repeated, tag = "2")]
    pub asset_chunks: Vec<SophonManifestAssetChunk>,

    /// 0 = file, 64 = directory.
    #[prost(uint32, tag = "3")]
    pub asset_type: u32,

    /// Uncompressed file size.
    #[prost(uint64, tag = "4")]
    pub asset_size: u64,

    /// MD5 of the assembled file.
    #[prost(string, tag = "5")]
    pub asset_hash_md5: String,
}

impl SophonManifestAssetProperty {
    /// Check if this asset is a directory.
    #[inline]
    pub fn is_directory(&self) -> bool {
        self.asset_type != 0 || self.asset_hash_md5.is_empty()
    }
}

// Chunk properties
#[derive(Clone, PartialEq, Message)]
pub struct SophonManifestAssetChunk {
    /// CDN object name for the download URL.
    #[prost(string, tag = "1")]
    pub chunk_name: String,

    /// MD5 of the decompressed chunk.
    #[prost(string, tag = "2")]
    pub chunk_decompressed_hash_md5: String,

    /// Offset in the output file. Defaults to 0 for the first chunk.
    #[prost(uint64, tag = "3")]
    pub chunk_on_file_offset: u64,

    /// Compressed chunk size from the CDN.
    #[prost(uint64, tag = "4")]
    pub chunk_size: u64,

    /// Decompressed chunk size.
    #[prost(uint64, tag = "5")]
    pub chunk_size_decompressed: u64,

    /// Undocumented hash field. Not used for verification.
    #[prost(uint64, tag = "6")]
    pub chunk_compressed_hash_xxh: u64,

    /// MD5 of the compressed chunk from the CDN.
    #[prost(string, tag = "7")]
    pub chunk_compressed_hash_md5: String,

    /// Runtime-only: -1 = new data, >= 0 = offset in the old file for chunk
    /// reuse.
    #[prost(int64, tag = "8")]
    pub chunk_old_offset: i64,
}

#[inline]
pub fn decode_manifest(buf: impl AsRef<[u8]>) -> Result<SophonManifestProto, prost::DecodeError> {
    SophonManifestProto::decode(buf.as_ref())
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

    #[prost(int64, tag = "2")]
    pub asset_size: i64,

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
pub fn decode_patch_manifest(
    buf: impl AsRef<[u8]>,
) -> Result<SophonPatchProto, prost::DecodeError> {
    SophonPatchProto::decode(buf.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_directory_type_nonzero_is_dir() {
        let prop = SophonManifestAssetProperty {
            asset_name: String::new(),
            asset_chunks: vec![],
            asset_type: 1,
            asset_size: 0,
            asset_hash_md5: "abc".into(),
        };
        assert!(prop.is_directory());
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
    fn is_directory_empty_hash_is_dir() {
        let prop = SophonManifestAssetProperty {
            asset_name: String::new(),
            asset_chunks: vec![],
            asset_type: 0,
            asset_size: 0,
            asset_hash_md5: String::new(),
        };
        assert!(prop.is_directory());
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
            chunk_old_offset: -1,
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

    #[test]
    fn decode_patch_manifest_valid() {
        let chunk = SophonPatchAssetChunk {
            patch_name: "diff_001.hdiff".into(),
            version_tag: "v3.0.0".into(),
            build_id: "build_123".into(),
            patch_size: 4096,
            patch_md5: "aabbccdd".into(),
            patch_offset: 0,
            patch_length: 4096,
            original_file_name: "data.bin".into(),
            original_file_length: 8192,
            original_file_md5: "11223344".into(),
        };
        let asset = SophonPatchAssetProperty {
            asset_name: "GameData/data.bin".into(),
            asset_size: 8192,
            asset_hash_md5: "11223344".into(),
            asset_infos: vec![SophonPatchAssetInfo {
                version_tag: "v3.0.0".into(),
                chunk: Some(chunk),
            }],
        };
        let unused_file = SophonUnusedAssetFile {
            file_name: "old_data.bin".into(),
            file_size: 1024,
            file_md5: "deadbeef".into(),
        };
        let unused_info = SophonUnusedAssetInfo {
            assets: vec![unused_file],
        };
        let unused_asset = SophonUnusedAssetProperty {
            version_tag: "v2.0.0".into(),
            asset_infos: vec![unused_info],
        };
        let original = SophonPatchProto {
            patch_assets: vec![asset],
            unused_assets: vec![unused_asset],
        };

        let buf = original.encode_to_vec();
        let decoded = decode_patch_manifest(&buf).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn decode_patch_manifest_invalid_garbage() {
        let result = decode_patch_manifest(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_patch_manifest_empty() {
        let buf = SophonPatchProto {
            patch_assets: vec![],
            unused_assets: vec![],
        }
        .encode_to_vec();
        let decoded = decode_patch_manifest(&buf).unwrap();
        assert!(decoded.patch_assets.is_empty());
        assert!(decoded.unused_assets.is_empty());
    }

    #[test]
    fn decode_patch_manifest_multiple_assets() {
        let make_asset = |name: &str, size: i64| SophonPatchAssetProperty {
            asset_name: name.into(),
            asset_size: size,
            asset_hash_md5: "hash".into(),
            asset_infos: vec![],
        };
        let original = SophonPatchProto {
            patch_assets: vec![
                make_asset("file_a.bin", 1000),
                make_asset("file_b.bin", 2000),
            ],
            unused_assets: vec![],
        };
        let buf = original.encode_to_vec();
        let decoded = decode_patch_manifest(&buf).unwrap();
        assert_eq!(decoded.patch_assets.len(), 2);
        assert_eq!(decoded.patch_assets[0].asset_name, "file_a.bin");
        assert_eq!(decoded.patch_assets[1].asset_name, "file_b.bin");
    }

    #[test]
    fn decode_patch_manifest_large_values() {
        let chunk = SophonPatchAssetChunk {
            patch_name: "big.hdiff".into(),
            version_tag: "v1".into(),
            build_id: "b1".into(),
            patch_size: i64::MAX,
            patch_md5: "md5".into(),
            patch_offset: i64::MAX,
            patch_length: i64::MAX,
            original_file_name: "big.bin".into(),
            original_file_length: i64::MAX,
            original_file_md5: "md5".into(),
        };
        let asset = SophonPatchAssetProperty {
            asset_name: "big.bin".into(),
            asset_size: i64::MAX,
            asset_hash_md5: "hash".into(),
            asset_infos: vec![SophonPatchAssetInfo {
                version_tag: "v1".into(),
                chunk: Some(chunk),
            }],
        };
        let original = SophonPatchProto {
            patch_assets: vec![asset],
            unused_assets: vec![],
        };
        let buf = original.encode_to_vec();
        let decoded = decode_patch_manifest(&buf).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn decode_patch_manifest_unused_only() {
        let unused = SophonUnusedAssetProperty {
            version_tag: "v1.0.0".into(),
            asset_infos: vec![SophonUnusedAssetInfo {
                assets: vec![SophonUnusedAssetFile {
                    file_name: "old.bin".into(),
                    file_size: 512,
                    file_md5: "abc".into(),
                }],
            }],
        };
        let original = SophonPatchProto {
            patch_assets: vec![],
            unused_assets: vec![unused],
        };
        let buf = original.encode_to_vec();
        let decoded = decode_patch_manifest(&buf).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn decode_patch_manifest_empty_strings() {
        let chunk = SophonPatchAssetChunk {
            patch_name: String::new(),
            version_tag: String::new(),
            build_id: String::new(),
            patch_size: 0,
            patch_md5: String::new(),
            patch_offset: 0,
            patch_length: 0,
            original_file_name: String::new(),
            original_file_length: 0,
            original_file_md5: String::new(),
        };
        let asset = SophonPatchAssetProperty {
            asset_name: String::new(),
            asset_size: 0,
            asset_hash_md5: String::new(),
            asset_infos: vec![SophonPatchAssetInfo {
                version_tag: String::new(),
                chunk: Some(chunk),
            }],
        };
        let original = SophonPatchProto {
            patch_assets: vec![asset],
            unused_assets: vec![],
        };
        let buf = original.encode_to_vec();
        let decoded = decode_patch_manifest(&buf).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn is_directory_type_0_nonempty_hash_is_not_dir() {
        let prop = SophonManifestAssetProperty {
            asset_name: "file.bin".into(),
            asset_chunks: vec![],
            asset_type: 0,
            asset_size: 1024,
            asset_hash_md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
        };
        assert!(!prop.is_directory());
    }

    #[test]
    fn is_directory_type_0_empty_hash_is_dir() {
        let prop = SophonManifestAssetProperty {
            asset_name: "somedir".into(),
            asset_chunks: vec![],
            asset_type: 0,
            asset_size: 0,
            asset_hash_md5: String::new(),
        };
        assert!(prop.is_directory());
    }

    #[test]
    fn decode_manifest_valid() {
        let original = SophonManifestProto {
            assets: vec![SophonManifestAssetProperty {
                asset_name: "Data/asset.pak".into(),
                asset_chunks: vec![SophonManifestAssetChunk {
                    chunk_name: "chunk_a".into(),
                    chunk_decompressed_hash_md5: "d1".into(),
                    chunk_on_file_offset: 0,
                    chunk_size: 500,
                    chunk_size_decompressed: 1000,
                    chunk_compressed_hash_xxh: 12345,
                    chunk_compressed_hash_md5: "c1".into(),
                    chunk_old_offset: -1,
                }],
                asset_type: 0,
                asset_size: 1000,
                asset_hash_md5: "a1".into(),
            }],
        };
        let buf = original.encode_to_vec();
        let decoded = decode_manifest(&buf).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn decode_manifest_multiple_assets() {
        let original = SophonManifestProto {
            assets: vec![
                SophonManifestAssetProperty {
                    asset_name: "file1.bin".into(),
                    asset_chunks: vec![],
                    asset_type: 0,
                    asset_size: 100,
                    asset_hash_md5: "h1".into(),
                },
                SophonManifestAssetProperty {
                    asset_name: "file2.bin".into(),
                    asset_chunks: vec![],
                    asset_type: 0,
                    asset_size: 200,
                    asset_hash_md5: "h2".into(),
                },
            ],
        };
        let buf = original.encode_to_vec();
        let decoded = decode_manifest(&buf).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn decode_manifest_chunk_with_old_offset_zero() {
        let original = SophonManifestProto {
            assets: vec![SophonManifestAssetProperty {
                asset_name: "file.bin".into(),
                asset_chunks: vec![SophonManifestAssetChunk {
                    chunk_name: "chunk_x".into(),
                    chunk_decompressed_hash_md5: String::new(),
                    chunk_on_file_offset: 0,
                    chunk_size: 1024,
                    chunk_size_decompressed: 2048,
                    chunk_compressed_hash_xxh: 0,
                    chunk_compressed_hash_md5: String::new(),
                    chunk_old_offset: 0,
                }],
                asset_type: 0,
                asset_size: 2048,
                asset_hash_md5: "h".into(),
            }],
        };
        let buf = original.encode_to_vec();
        let decoded = decode_manifest(&buf).unwrap();
        assert_eq!(original, decoded);
        assert_eq!(decoded.assets[0].asset_chunks[0].chunk_old_offset, 0);
    }

    #[test]
    fn decode_manifest_chunk_with_positive_old_offset() {
        let original = SophonManifestProto {
            assets: vec![SophonManifestAssetProperty {
                asset_name: "file.bin".into(),
                asset_chunks: vec![SophonManifestAssetChunk {
                    chunk_name: "chunk_y".into(),
                    chunk_decompressed_hash_md5: String::new(),
                    chunk_on_file_offset: 100,
                    chunk_size: 512,
                    chunk_size_decompressed: 1024,
                    chunk_compressed_hash_xxh: 0,
                    chunk_compressed_hash_md5: String::new(),
                    chunk_old_offset: 200,
                }],
                asset_type: 0,
                asset_size: 2048,
                asset_hash_md5: "h".into(),
            }],
        };
        let buf = original.encode_to_vec();
        let decoded = decode_manifest(&buf).unwrap();
        assert_eq!(original, decoded);
        assert_eq!(decoded.assets[0].asset_chunks[0].chunk_old_offset, 200);
    }
}
