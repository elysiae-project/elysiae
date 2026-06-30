//! Columnar manifest storage.
//!
//! Replaces the protobuf `Vec<SophonManifestAssetProperty>` representation
//! with per-field `Vec` columns and a shared string arena. This eliminates
//! per-string and per-Vec heap allocation overhead and improves jemalloc
//! fragmentation by co-locating fields in contiguous allocations.

use crate::commands::sophon_downloader::proto_parse::{
    SophonManifestAssetChunk, SophonManifestAssetProperty,
};

impl<'a> From<&'a SophonManifestAssetChunk> for ChunkRef<'a> {
    fn from(chunk: &'a SophonManifestAssetChunk) -> Self {
        ChunkRef {
            chunk_name: &chunk.chunk_name,
            chunk_decompressed_hash_md5: &chunk.chunk_decompressed_hash_md5,
            chunk_on_file_offset: chunk.chunk_on_file_offset,
            chunk_size: chunk.chunk_size,
            chunk_size_decompressed: chunk.chunk_size_decompressed,
            chunk_compressed_hash_md5: &chunk.chunk_compressed_hash_md5,
            chunk_old_offset: chunk.chunk_old_offset,
        }
    }
}

/// Shared string arena. All manifest strings are concatenated into a single
/// `String` and referenced by `(offset, len)` spans.
#[derive(Default)]
pub struct StringArena {
    data: String,
    spans: Vec<(u32, u32)>,
}

impl StringArena {
    pub fn with_capacity(spans: usize, total_bytes: usize) -> Self {
        Self {
            data: String::with_capacity(total_bytes),
            spans: Vec::with_capacity(spans),
        }
    }

    pub fn intern(&mut self, s: &str) -> u32 {
        let idx = self.spans.len() as u32;
        let offset = self.data.len() as u32;
        let len = s.len() as u32;
        self.data.push_str(s);
        self.spans.push((offset, len));
        idx
    }

    #[inline]
    pub fn get(&self, idx: u32) -> &str {
        let (offset, len) = self.spans[idx as usize];
        &self.data[offset as usize..(offset + len) as usize]
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    #[inline]
    pub fn byte_len(&self) -> usize {
        self.data.len()
    }
}

/// Zero-copy view into a single chunk row in `CompactManifest`.
#[derive(Clone, Copy)]
pub struct ChunkRef<'a> {
    pub chunk_name: &'a str,
    pub chunk_decompressed_hash_md5: &'a str,
    pub chunk_on_file_offset: u64,
    pub chunk_size: u64,
    pub chunk_size_decompressed: u64,
    pub chunk_compressed_hash_md5: &'a str,
    pub chunk_old_offset: i64,
}

/// Columnar manifest storage.
///
/// Chunks are stored in a single flat array indexed globally. Each file
/// owns a contiguous slice `[file_chunk_start[i]..file_chunk_start[i+1])`.
pub struct CompactManifest {
    arena: StringArena,
    file_name_idx: Vec<u32>,
    file_hash_idx: Vec<u32>,
    file_type: Vec<u32>,
    file_size: Vec<u64>,
    file_chunk_start: Vec<u32>,
    chunk_name_idx: Vec<u32>,
    chunk_decomp_hash_idx: Vec<u32>,
    chunk_comp_hash_idx: Vec<u32>,
    chunk_on_file_offset: Vec<u64>,
    chunk_size: Vec<u64>,
    chunk_size_decompressed: Vec<u64>,
    chunk_old_offset: Vec<i64>,
}

impl CompactManifest {
    pub fn num_files(&self) -> usize {
        self.file_name_idx.len()
    }

    pub fn num_chunks(&self) -> usize {
        self.chunk_name_idx.len()
    }

    #[inline]
    pub fn file_name(&self, file_idx: usize) -> &str {
        self.arena.get(self.file_name_idx[file_idx])
    }

    #[inline]
    pub fn file_hash_md5(&self, file_idx: usize) -> &str {
        self.arena.get(self.file_hash_idx[file_idx])
    }

    #[inline]
    pub fn file_type(&self, file_idx: usize) -> u32 {
        self.file_type[file_idx]
    }

    #[inline]
    pub fn file_size(&self, file_idx: usize) -> u64 {
        self.file_size[file_idx]
    }

    #[inline]
    pub fn is_directory(&self, file_idx: usize) -> bool {
        self.file_type[file_idx] != 0 || self.file_hash_md5(file_idx).is_empty()
    }

    #[inline]
    pub fn file_chunk_range(&self, file_idx: usize) -> std::ops::Range<u32> {
        let start = self.file_chunk_start[file_idx];
        let end = self
            .file_chunk_start
            .get(file_idx + 1)
            .copied()
            .unwrap_or(self.chunk_name_idx.len() as u32);
        start..end
    }

    #[inline]
    pub fn chunk(&self, chunk_idx: usize) -> ChunkRef<'_> {
        ChunkRef {
            chunk_name: self.arena.get(self.chunk_name_idx[chunk_idx]),
            chunk_decompressed_hash_md5: self.arena.get(self.chunk_decomp_hash_idx[chunk_idx]),
            chunk_on_file_offset: self.chunk_on_file_offset[chunk_idx],
            chunk_size: self.chunk_size[chunk_idx],
            chunk_size_decompressed: self.chunk_size_decompressed[chunk_idx],
            chunk_compressed_hash_md5: self.arena.get(self.chunk_comp_hash_idx[chunk_idx]),
            chunk_old_offset: self.chunk_old_offset[chunk_idx],
        }
    }

    /// Access a chunk by its `(file_idx, chunk_idx_in_file)` pair.
    #[inline]
    pub fn file_chunk(&self, file_idx: usize, chunk_idx_in_file: usize) -> ChunkRef<'_> {
        let range = self.file_chunk_range(file_idx);
        self.chunk(range.start as usize + chunk_idx_in_file)
    }

    /// Arena byte size for memory logging.
    pub fn arena_bytes(&self) -> usize {
        self.arena.byte_len() + self.arena.len() * std::mem::size_of::<(u32, u32)>()
    }

    /// Columnar byte size for memory logging.
    pub fn column_bytes(&self) -> usize {
        let n_files = self.file_name_idx.len();
        let n_chunks = self.chunk_name_idx.len();
        n_files * (4 + 4 + 4 + 8 + 4)
            + n_chunks * (4 + 4 + 4 + 8 + 8 + 8 + 8)
            + n_files * std::mem::size_of::<u32>() * 2
            + 7 * std::mem::size_of::<Vec<u8>>()
    }
}

impl From<Vec<SophonManifestAssetProperty>> for CompactManifest {
    fn from(properties: Vec<SophonManifestAssetProperty>) -> Self {
        let total_files = properties.len();
        let total_chunks: usize = properties.iter().map(|f| f.asset_chunks.len()).sum();
        let estimated_string_bytes: usize = properties
            .iter()
            .map(|f| {
                f.asset_name.len()
                    + f.asset_hash_md5.len()
                    + f.asset_chunks
                        .iter()
                        .map(|c| c.chunk_name.len())
                        .sum::<usize>()
                    + f.asset_chunks
                        .iter()
                        .map(|c| c.chunk_decompressed_hash_md5.len())
                        .sum::<usize>()
                    + f.asset_chunks
                        .iter()
                        .map(|c| c.chunk_compressed_hash_md5.len())
                        .sum::<usize>()
            })
            .sum();

        let mut arena =
            StringArena::with_capacity(total_files * 2 + total_chunks * 3, estimated_string_bytes);
        let mut file_name_idx = Vec::with_capacity(total_files);
        let mut file_hash_idx = Vec::with_capacity(total_files);
        let mut file_type = Vec::with_capacity(total_files);
        let mut file_size = Vec::with_capacity(total_files);
        let mut file_chunk_start = Vec::with_capacity(total_files + 1);

        let mut chunk_name_idx = Vec::with_capacity(total_chunks);
        let mut chunk_decomp_hash_idx = Vec::with_capacity(total_chunks);
        let mut chunk_comp_hash_idx = Vec::with_capacity(total_chunks);
        let mut chunk_on_file_offset = Vec::with_capacity(total_chunks);
        let mut chunk_size = Vec::with_capacity(total_chunks);
        let mut chunk_size_decompressed = Vec::with_capacity(total_chunks);
        let mut chunk_old_offset = Vec::with_capacity(total_chunks);

        for file in &properties {
            file_name_idx.push(arena.intern(&file.asset_name));
            file_hash_idx.push(arena.intern(&file.asset_hash_md5));
            file_type.push(file.asset_type);
            file_size.push(file.asset_size);
            file_chunk_start.push(chunk_name_idx.len() as u32);
            for chunk in &file.asset_chunks {
                chunk_name_idx.push(arena.intern(&chunk.chunk_name));
                chunk_decomp_hash_idx.push(arena.intern(&chunk.chunk_decompressed_hash_md5));
                chunk_comp_hash_idx.push(arena.intern(&chunk.chunk_compressed_hash_md5));
                chunk_on_file_offset.push(chunk.chunk_on_file_offset);
                chunk_size.push(chunk.chunk_size);
                chunk_size_decompressed.push(chunk.chunk_size_decompressed);
                chunk_old_offset.push(chunk.chunk_old_offset);
            }
        }

        Self {
            arena,
            file_name_idx,
            file_hash_idx,
            file_type,
            file_size,
            file_chunk_start,
            chunk_name_idx,
            chunk_decomp_hash_idx,
            chunk_comp_hash_idx,
            chunk_on_file_offset,
            chunk_size,
            chunk_size_decompressed,
            chunk_old_offset,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetChunk;

    fn make_chunk(
        name: &str,
        chunk_on_file_offset: u64,
        size: u64,
        decompressed: u64,
    ) -> SophonManifestAssetChunk {
        SophonManifestAssetChunk {
            chunk_name: name.into(),
            chunk_decompressed_hash_md5: String::new(),
            chunk_on_file_offset,
            chunk_size: size,
            chunk_size_decompressed: decompressed,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: String::new(),
            chunk_old_offset: -1,
        }
    }

    fn make_file(
        name: &str,
        md5: &str,
        asset_type: u32,
        asset_size: u64,
        chunks: Vec<SophonManifestAssetChunk>,
    ) -> SophonManifestAssetProperty {
        SophonManifestAssetProperty {
            asset_name: name.into(),
            asset_chunks: chunks,
            asset_type,
            asset_size,
            asset_hash_md5: md5.into(),
        }
    }

    #[test]
    fn empty_manifest_has_zero_files_and_chunks() {
        let cm = CompactManifest::from(vec![]);
        assert_eq!(cm.num_files(), 0);
        assert_eq!(cm.num_chunks(), 0);
    }

    #[test]
    fn single_file_no_chunks() {
        let file = make_file("a.pak", "d41d8cd98f00b204e9800998ecf8427e", 0, 1024, vec![]);
        let cm = CompactManifest::from(vec![file]);
        assert_eq!(cm.num_files(), 1);
        assert_eq!(cm.num_chunks(), 0);
        assert_eq!(cm.file_name(0), "a.pak");
        assert_eq!(cm.file_hash_md5(0), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(cm.file_size(0), 1024);
        assert!(!cm.is_directory(0));
        assert_eq!(cm.file_chunk_range(0), 0..0);
    }

    #[test]
    fn directory_detected_by_empty_hash() {
        let dir = make_file("GameData", "", 0, 0, vec![]);
        let cm = CompactManifest::from(vec![dir]);
        assert!(cm.is_directory(0));
    }

    #[test]
    fn directory_detected_by_nonzero_type() {
        let dir = make_file("GameData", "abc", 64, 0, vec![]);
        let cm = CompactManifest::from(vec![dir]);
        assert!(cm.is_directory(0));
    }

    #[test]
    fn multiple_files_with_chunks_have_correct_ranges() {
        let f1 = make_file(
            "a.pak",
            "h1",
            0,
            300,
            vec![
                make_chunk("c1", 0, 100, 100),
                make_chunk("c2", 100, 100, 200),
            ],
        );
        let f2 = make_file("b.pak", "h2", 0, 500, vec![make_chunk("c3", 0, 500, 500)]);
        let cm = CompactManifest::from(vec![f1, f2]);
        assert_eq!(cm.num_files(), 2);
        assert_eq!(cm.num_chunks(), 3);
        assert_eq!(cm.file_chunk_range(0), 0..2);
        assert_eq!(cm.file_chunk_range(1), 2..3);
    }

    #[test]
    fn file_chunk_accessor_returns_correct_data() {
        let f1 = make_file(
            "a.pak",
            "h1",
            0,
            300,
            vec![
                make_chunk("c1", 0, 100, 200),
                make_chunk("c2", 200, 100, 300),
            ],
        );
        let cm = CompactManifest::from(vec![f1]);
        let c = cm.file_chunk(0, 0);
        assert_eq!(c.chunk_name, "c1");
        assert_eq!(c.chunk_size, 100);
        assert_eq!(c.chunk_size_decompressed, 200);
        assert_eq!(c.chunk_on_file_offset, 0);
        let c = cm.file_chunk(0, 1);
        assert_eq!(c.chunk_name, "c2");
        assert_eq!(c.chunk_on_file_offset, 200);
    }

    #[test]
    fn chunk_compressed_hash_md5_preserved() {
        let mut chunk = make_chunk("c1", 0, 100, 100);
        chunk.chunk_compressed_hash_md5 = "abcdef1234567890abcdef1234567890".into();
        chunk.chunk_decompressed_hash_md5 = "00000000000000000000000000000000".into();
        let f = make_file("a.pak", "h", 0, 100, vec![chunk]);
        let cm = CompactManifest::from(vec![f]);
        let c = cm.chunk(0);
        assert_eq!(
            c.chunk_compressed_hash_md5,
            "abcdef1234567890abcdef1234567890"
        );
        assert_eq!(
            c.chunk_decompressed_hash_md5,
            "00000000000000000000000000000000"
        );
    }

    #[test]
    fn chunk_old_offset_preserved() {
        let mut chunk = make_chunk("c1", 0, 100, 100);
        chunk.chunk_old_offset = 42;
        let f = make_file("a.pak", "h", 0, 100, vec![chunk]);
        let cm = CompactManifest::from(vec![f]);
        assert_eq!(cm.chunk(0).chunk_old_offset, 42);
    }

    #[test]
    fn arena_stores_all_strings() {
        let f1 = make_file("a.pak", "shared", 0, 100, vec![]);
        let f2 = make_file("b.pak", "shared", 0, 200, vec![]);
        let cm = CompactManifest::from(vec![f1, f2]);
        assert_eq!(cm.arena.len(), 4);
    }
}
