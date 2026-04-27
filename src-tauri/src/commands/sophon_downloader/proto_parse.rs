// Protobuf data structure definition for Sophon manifests
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
    #[inline]
    pub fn is_directory(&self) -> bool {
        self.asset_type != 0 || self.asset_hash_md5.is_empty()
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
