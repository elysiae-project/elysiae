#[allow(dead_code)]
mod compression;
mod parser;
mod patch_core;
mod patch_sf;
mod patch_single;

use std::fs::File;
use std::io::{BufWriter, Read, Seek, Write};

use parser::BinaryExtensions;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CompressionMode {
    #[default]
    Nocomp,
    Zstd,
    Zlib,
}

impl std::str::FromStr for CompressionMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "" | "nocomp" => Ok(CompressionMode::Nocomp),
            "zstd" => Ok(CompressionMode::Zstd),
            _ => Err(format!("unsupported compression mode: {s}")),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct HeaderInfo {
    comp_mode: CompressionMode,
    is_single_compressed_diff: bool,
    patch_path: String,
    step_mem_size: i64,
    old_data_size: i64,
    new_data_size: i64,
    compressed_count: i64,
    single_chunk_info: DiffSingleChunkInfo,
    chunk_info: DiffChunkInfo,
}

#[derive(Debug, Clone, Default)]
struct DiffSingleChunkInfo {
    uncompressed_size: i64,
    compressed_size: i64,
    diff_data_pos: i64,
}

#[derive(Debug, Clone, Default)]
struct DiffChunkInfo {
    types_end_pos: i64,
    cover_count: i64,
    cover_buf_size: i64,
    compress_cover_buf_size: i64,
    rle_ctrl_buf_size: i64,
    compress_rle_ctrl_buf_size: i64,
    rle_code_buf_size: i64,
    compress_rle_code_buf_size: i64,
    new_data_diff_size: i64,
    compress_new_data_diff_size: i64,
    head_end_pos: i64,
    cover_end_pos: i64,
}

#[derive(Debug, Clone, Copy, Default)]
struct RleRefClip {
    mem_copy_length: i64,
    mem_set_length: i64,
    mem_set_value: u8,
}

#[derive(Debug, Clone, Copy)]
struct CoverHeader {
    old_pos: i64,
    new_pos: i64,
    cover_length: i64,
    next_cover_index: i64,
}

impl CoverHeader {
    fn new(old_pos: i64, new_pos: i64, cover_length: i64, next_cover_index: i64) -> Self {
        Self {
            old_pos,
            new_pos,
            cover_length,
            next_cover_index,
        }
    }
}

const K_SIGN_TAG_BIT: u8 = 1;
const K_BYTE_RLE_TYPE: u8 = 2;
const MAX_MEM_BUFFER_LEN: i64 = 7 << 20;
const MAX_MEM_BUFFER_LIMIT: usize = 10 << 20;
const MAX_ARRAY_POOL_LEN: usize = 4 << 20;
const MAX_ARRAY_POOL_SECOND_OFFSET: usize = MAX_ARRAY_POOL_LEN / 2;

pub struct HDiff {
    source_path: String,
    diff_path: String,
    dest_path: String,
}

impl HDiff {
    pub fn new(source_path: String, diff_path: String, dest_path: String) -> Self {
        HDiff {
            source_path,
            diff_path,
            dest_path,
        }
    }

    pub fn apply(&mut self) -> bool {
        match self.apply_inner() {
            Ok(()) => true,
            Err(e) => {
                tauri_plugin_log::log::error!("[HDiff::apply] Error: {e}");
                false
            }
        }
    }

    fn apply_inner(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut diff_file = File::open(&self.diff_path)?;
        let mut header_info = HeaderInfo::default();
        let header_info_line = diff_file.read_string_to_null(512)?;

        if header_info_line.len() > 64 || !header_info_line.starts_with("HDIFF") {
            return Err("not a HDiff file format".into());
        }
        let h_info_arr: Vec<&str> = header_info_line.split('&').collect();
        if h_info_arr.len() != 2 {
            return Err(format!(
                "unsupported HDiff header format: expected 2 parts, got {} (raw: {})",
                h_info_arr.len(),
                header_info_line
            )
            .into());
        }

        let p_file_ver = Self::try_get_version(h_info_arr[0])?;
        if p_file_ver != 13 && p_file_ver != 20 {
            return Err(format!(
                "unsupported HDiff version {p_file_ver} (only 13 and 20 supported)"
            )
            .into());
        }

        header_info.comp_mode = h_info_arr[1].parse()?;
        header_info.is_single_compressed_diff = p_file_ver == 20;
        header_info.patch_path = self.diff_path.clone();

        if header_info.is_single_compressed_diff {
            Self::read_single_file_header(&mut diff_file, &mut header_info)?;
        } else {
            Self::read_non_single_file_header(&mut diff_file, &mut header_info)?;
        }

        let mut old_file = File::open(&self.source_path)?;
        let old_len = old_file.metadata()?.len() as i64;
        if old_len != header_info.old_data_size {
            return Err(format!(
                "input file size mismatch: expected {} bytes, got {} bytes",
                header_info.old_data_size, old_len
            )
            .into());
        }

        let out_file = File::create(&self.dest_path)?;
        let mut out_writer = BufWriter::new(out_file);

        if header_info.is_single_compressed_diff {
            patch_sf::PatchSF::new(header_info).patch(
                &mut old_file,
                &mut out_writer,
                &self.diff_path,
            )?;
        } else {
            patch_single::PatchSingle::new(header_info).patch(
                &mut old_file,
                &mut out_writer,
                &self.diff_path,
            )?;
        }
        out_writer.flush()?;
        Ok(())
    }

    fn try_get_version(str_val: &str) -> Result<i64, Box<dyn std::error::Error>> {
        let idx = str_val
            .find("HDIFF")
            .ok_or_else(|| format!("cannot find 'HDIFF' in: {str_val}"))?;
        let rest = &str_val[idx + "HDIFF".len()..];
        let num_str = rest.trim_start_matches(|c: char| !c.is_ascii_digit());
        num_str
            .parse::<i64>()
            .map_err(|_| format!("invalid version string: {num_str} (raw: {str_val})").into())
    }

    fn read_single_file_header(
        sr: &mut (impl Read + Seek),
        header_info: &mut HeaderInfo,
    ) -> Result<(), Box<dyn std::error::Error>> {
        header_info.single_chunk_info = DiffSingleChunkInfo::default();
        header_info.new_data_size = sr.read_long_7bit()?;
        header_info.old_data_size = sr.read_long_7bit()?;

        header_info.chunk_info.cover_count = sr.read_long_7bit()?;
        header_info.step_mem_size = sr.read_long_7bit()?;
        header_info.single_chunk_info.uncompressed_size = sr.read_long_7bit()?;
        header_info.single_chunk_info.compressed_size = sr.read_long_7bit()?;

        let pos = sr.stream_position()? as i64;
        header_info.single_chunk_info.diff_data_pos = pos;
        header_info.compressed_count = if header_info.single_chunk_info.compressed_size > 0 {
            1
        } else {
            0
        };
        Ok(())
    }

    fn read_non_single_file_header(
        sr: &mut (impl Read + Seek),
        header_info: &mut HeaderInfo,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let type_end_pos = sr.stream_position()? as i64;
        header_info.new_data_size = sr.read_long_7bit()?;
        header_info.old_data_size = sr.read_long_7bit()?;

        Self::get_diff_chunk_info(sr, &mut header_info.chunk_info, type_end_pos)?;
        header_info.compressed_count = ((header_info.chunk_info.compress_cover_buf_size > 1)
            as i64)
            + ((header_info.chunk_info.compress_rle_ctrl_buf_size > 1) as i64)
            + ((header_info.chunk_info.compress_rle_code_buf_size > 1) as i64)
            + ((header_info.chunk_info.compress_new_data_diff_size > 1) as i64);
        Ok(())
    }

    fn get_diff_chunk_info(
        sr: &mut (impl Read + Seek),
        chunk_info: &mut DiffChunkInfo,
        type_end_pos: i64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        *chunk_info = DiffChunkInfo::default();
        chunk_info.types_end_pos = type_end_pos;
        chunk_info.cover_count = sr.read_long_7bit()?;
        chunk_info.cover_buf_size = sr.read_long_7bit()?;
        chunk_info.compress_cover_buf_size = sr.read_long_7bit()?;
        chunk_info.rle_ctrl_buf_size = sr.read_long_7bit()?;
        chunk_info.compress_rle_ctrl_buf_size = sr.read_long_7bit()?;
        chunk_info.rle_code_buf_size = sr.read_long_7bit()?;
        chunk_info.compress_rle_code_buf_size = sr.read_long_7bit()?;
        chunk_info.new_data_diff_size = sr.read_long_7bit()?;
        chunk_info.compress_new_data_diff_size = sr.read_long_7bit()?;

        chunk_info.head_end_pos = sr.stream_position()? as i64;
        chunk_info.cover_end_pos = chunk_info.head_end_pos
            + if chunk_info.compress_cover_buf_size > 0 {
                chunk_info.compress_cover_buf_size
            } else {
                chunk_info.cover_buf_size
            };
        Ok(())
    }
}

trait SeekableRead: Read + Seek {}
impl<T: Read + Seek> SeekableRead for T {}

#[cfg(test)]
mod tests {
    use super::HDiff;
    use std::fs;

    fn setup_test_patches() -> &'static str {
        "/tmp/hdiff_test"
    }

    #[test]
    fn hdiff_v13_zstd_text_patch() {
        let dir = setup_test_patches();
        let old = format!("{}/old.bin", dir);
        let diff = format!("{}/v13_zstd.hdiff", dir);
        let out = format!("{}/output_v13.bin", dir);

        let mut hdiff = HDiff::new(old, diff, out.clone());
        assert!(hdiff.apply(), "HDiff apply failed for v13 zstd text patch");

        let result = fs::read(&out).unwrap();
        let expected = fs::read(format!("{}/expected_new.bin", dir)).unwrap();
        assert_eq!(result, expected, "Patched output doesn't match expected");
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn hdiff_v13_zstd_empty_original() {
        let dir = setup_test_patches();
        let old = format!("{}/empty.bin", dir);
        let diff = format!("{}/empty_to_new.hdiff", dir);
        let out = format!("{}/output_empty.bin", dir);

        let mut hdiff = HDiff::new(old, diff, out.clone());
        assert!(hdiff.apply(), "HDiff apply failed for empty original patch");

        let result = fs::read(&out).unwrap();
        let expected = fs::read(format!("{}/expected_new_from_empty.bin", dir)).unwrap();
        assert_eq!(
            result, expected,
            "Patched output from empty doesn't match expected"
        );
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn hdiff_v13_zstd_large_binary() {
        let dir = setup_test_patches();
        let old = format!("{}/old_large.bin", dir);
        let diff = format!("{}/large_zstd.hdiff", dir);
        let out = format!("{}/output_large.bin", dir);

        let mut hdiff = HDiff::new(old, diff, out.clone());
        assert!(hdiff.apply(), "HDiff apply failed for large binary patch");

        let result = fs::read(&out).unwrap();
        let expected = fs::read(format!("{}/expected_new_large.bin", dir)).unwrap();
        assert_eq!(
            result, expected,
            "Patched large binary doesn't match expected"
        );
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn hdiff_v13_zstd_identity_patch() {
        let dir = setup_test_patches();
        let old = format!("{}/same.bin", dir);
        let diff = format!("{}/identity.hdiff", dir);
        let out = format!("{}/output_identity.bin", dir);

        let mut hdiff = HDiff::new(old, diff, out.clone());
        assert!(hdiff.apply(), "HDiff apply failed for identity patch");

        let result = fs::read(&out).unwrap();
        let expected = fs::read(format!("{}/same.bin", dir)).unwrap();
        assert_eq!(
            result, expected,
            "Identity patch output doesn't match original"
        );
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn hdiff_wrong_old_size_fails() {
        let dir = setup_test_patches();
        let wrong_old = format!("{}/same.bin", dir);
        let diff = format!("{}/v13_zstd.hdiff", dir);
        let out = format!("{}/output_wrong.bin", dir);

        let mut hdiff = HDiff::new(wrong_old, diff, out.clone());
        assert!(
            !hdiff.apply(),
            "HDiff should fail when old file size doesn't match"
        );
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn hdiff_invalid_diff_file_fails() {
        let dir = setup_test_patches();
        let tmp = tempfile::tempdir().unwrap();
        let fake_diff = tmp.path().join("fake.hdiff");
        fs::write(&fake_diff, b"NOT_A_HDIFF_FILE_CONTENTS").unwrap();
        let old = format!("{}/old.bin", dir);
        let out = tmp.path().join("output.bin").to_string_lossy().to_string();

        let mut hdiff = HDiff::new(old, fake_diff.to_string_lossy().to_string(), out);
        assert!(!hdiff.apply(), "HDiff should fail for invalid diff file");
    }

    #[test]
    fn hdiff_detect_hdiff_magic_bytes() {
        let dir = setup_test_patches();
        let diff = format!("{}/v13_zstd.hdiff", dir);
        let data = fs::read(&diff).unwrap();
        assert!(
            data.starts_with(b"HDIFF"),
            "Diff file should start with HDIFF magic bytes"
        );
    }
}
