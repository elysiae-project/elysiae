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
    Lz4,
}

impl std::str::FromStr for CompressionMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "" | "nocomp" => Ok(CompressionMode::Nocomp),
            "zstd" => Ok(CompressionMode::Zstd),
            "zlib" => Ok(CompressionMode::Zlib),
            "lz4" => Ok(CompressionMode::Lz4),
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
        let expected_size = header_info.new_data_size;

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

        if expected_size < 0 {
            return Err("new_data_size is negative".into());
        }
        let actual_size = std::fs::metadata(&self.dest_path)?.len() as i64;
        if actual_size != expected_size {
            return Err(format!(
                "Patch output size mismatch: expected {} bytes, got {} bytes",
                expected_size, actual_size
            )
            .into());
        }

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
        if header_info.new_data_size < 0 || header_info.old_data_size < 0 {
            return Err("new_data_size or old_data_size is negative".into());
        }

        header_info.chunk_info.cover_count = sr.read_long_7bit()?;
        header_info.step_mem_size = sr.read_long_7bit()?;
        if header_info.chunk_info.cover_count < 0 {
            return Err("cover_count is negative".into());
        }
        if header_info.step_mem_size < 0 {
            return Err("step_mem_size is negative".into());
        }
        header_info.single_chunk_info.uncompressed_size = sr.read_long_7bit()?;
        header_info.single_chunk_info.compressed_size = sr.read_long_7bit()?;
        if header_info.single_chunk_info.uncompressed_size < 0 {
            return Err("uncompressed_size is negative".into());
        }
        if header_info.single_chunk_info.compressed_size < 0 {
            return Err("compressed_size is negative".into());
        }

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
        if header_info.new_data_size < 0 || header_info.old_data_size < 0 {
            return Err("new_data_size or old_data_size is negative".into());
        }

        Self::get_diff_chunk_info(sr, &mut header_info.chunk_info, type_end_pos)?;
        header_info.compressed_count = ((header_info.chunk_info.compress_cover_buf_size > 0)
            as i64)
            + ((header_info.chunk_info.compress_rle_ctrl_buf_size > 0) as i64)
            + ((header_info.chunk_info.compress_rle_code_buf_size > 0) as i64)
            + ((header_info.chunk_info.compress_new_data_diff_size > 0) as i64);
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

        let fields: &[(&str, i64)] = &[
            ("cover_buf_size", chunk_info.cover_buf_size),
            (
                "compress_cover_buf_size",
                chunk_info.compress_cover_buf_size,
            ),
            ("rle_ctrl_buf_size", chunk_info.rle_ctrl_buf_size),
            (
                "compress_rle_ctrl_buf_size",
                chunk_info.compress_rle_ctrl_buf_size,
            ),
            ("rle_code_buf_size", chunk_info.rle_code_buf_size),
            (
                "compress_rle_code_buf_size",
                chunk_info.compress_rle_code_buf_size,
            ),
            ("new_data_diff_size", chunk_info.new_data_diff_size),
            (
                "compress_new_data_diff_size",
                chunk_info.compress_new_data_diff_size,
            ),
        ];
        for (name, val) in fields {
            if *val < 0 {
                return Err(format!("{} is negative in diff chunk info", name).into());
            }
        }

        chunk_info.head_end_pos = sr.stream_position()? as i64;
        chunk_info.cover_end_pos = chunk_info
            .head_end_pos
            .checked_add(if chunk_info.compress_cover_buf_size > 0 {
                chunk_info.compress_cover_buf_size
            } else {
                chunk_info.cover_buf_size
            })
            .ok_or_else(|| -> Box<dyn std::error::Error> {
                "cover_end_pos overflow in diff chunk info".into()
            })?;
        Ok(())
    }
}

trait SeekableRead: Read + Seek {}
impl<T: Read + Seek> SeekableRead for T {}

#[cfg(test)]
mod tests {
    use super::HDiff;
    use std::fs;

    const OLD_TEXT: &[u8] =
        b"Hello World! This is the original file content for testing HDiff patching.";
    const NEW_TEXT: &[u8] =
        b"Hello Universe! This is the modified file content for testing HDiff patching.";
    const DIFF_V13_TEXT: &[u8] = b"HDIFF13&zstd\x00MJ\x01\x03\x00\x04\x00\x08\x00\x0e\x00\x0b\x0e?\x1b\xc7 (\xfe\xfd\xfb\x02\xfd\xfb\x04\xf8Hello Universe";

    const NEW_FROM_EMPTY: &[u8] = b"This is brand new content created from nothing!";
    const DIFF_EMPTY_TO_NEW: &[u8] = b"HDIFF13&zstd\x00/\x00\x00\x00\x00\x02\x00\x00\x00/\x00 .This is brand new content created from nothing!";

    const DIFF_LARGE: &[u8] = b"HDIFF13&zstd\x00\x84\x80\x00\x84\x80\x00\x01\x05\x00\xe63\x16\xb3\x19\x12\x01\x00\x01\x01\x83\xff\x7f(\xb5/\xfd`32e\x00\x00 \t\x80\x08\x04\x01\x00,3\xde\r\x01(\xb5/\xfd`\x99\x18E\x00\x00\x08\x01\x01\x00\x95\xd9\x03!:";

    const SAME: &[u8] = b"Identical content on both sides";
    const DIFF_IDENTITY: &[u8] =
        b"HDIFF13&zstd\x00\x1f\x1f\x01\x03\x00\x01\x00\x00\x00\x00\x00\x00\x00\x1f\x1e";

    fn write_and_apply(old: &[u8], diff: &[u8], expected: &[u8]) -> bool {
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("old.bin");
        let diff_path = dir.path().join("diff.hdiff");
        let out_path = dir.path().join("out.bin");

        fs::write(&old_path, old).unwrap();
        fs::write(&diff_path, diff).unwrap();

        let op = old_path.to_string_lossy().to_string();
        let dp = diff_path.to_string_lossy().to_string();
        let tp = out_path.to_string_lossy().to_string();

        let mut hdiff = HDiff::new(op, dp, tp);
        if !hdiff.apply() {
            return false;
        }

        let result = fs::read(&out_path).unwrap();
        result == expected
    }

    #[test]
    fn hdiff_v13_zstd_text_patch() {
        assert!(
            write_and_apply(OLD_TEXT, DIFF_V13_TEXT, NEW_TEXT),
            "v13 zstd text patch output mismatch"
        );
    }

    #[test]
    fn hdiff_v13_zstd_empty_original() {
        assert!(
            write_and_apply(b"", DIFF_EMPTY_TO_NEW, NEW_FROM_EMPTY),
            "empty original patch output mismatch"
        );
    }

    #[test]
    fn hdiff_v13_zstd_large_binary() {
        let fixture_dir = "/tmp/hdiff_test";
        let old_path = format!("{}/old_large.bin", fixture_dir);
        let diff_path = format!("{}/large_zstd.hdiff", fixture_dir);
        let out_path = format!("{}/output_large_test.bin", fixture_dir);

        if !std::path::Path::new(&old_path).exists() {
            eprintln!("skipping: large binary test fixtures not present at {fixture_dir}");
            return;
        }

        let mut hdiff = HDiff::new(old_path, diff_path, out_path.clone());
        assert!(hdiff.apply(), "large binary patch apply failed");

        let result = fs::read(&out_path).unwrap();
        let expected = fs::read(format!("{}/new_large.bin", fixture_dir)).unwrap();
        assert_eq!(result, expected, "large binary patch output mismatch");
        let _ = fs::remove_file(&out_path);
    }

    #[test]
    fn hdiff_v13_zstd_identity_patch() {
        assert!(
            write_and_apply(SAME, DIFF_IDENTITY, SAME),
            "identity patch output mismatch"
        );
    }

    #[test]
    fn hdiff_wrong_old_size_fails() {
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("old.bin");
        let diff_path = dir.path().join("diff.hdiff");
        let out_path = dir.path().join("out.bin");

        fs::write(&old_path, SAME).unwrap();
        fs::write(&diff_path, DIFF_V13_TEXT).unwrap();

        let op = old_path.to_string_lossy().to_string();
        let dp = diff_path.to_string_lossy().to_string();
        let tp = out_path.to_string_lossy().to_string();

        let mut hdiff = HDiff::new(op, dp, tp);
        assert!(
            !hdiff.apply(),
            "should fail when old file size doesn't match"
        );
    }

    #[test]
    fn hdiff_invalid_diff_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("old.bin");
        let diff_path = dir.path().join("diff.hdiff");
        let out_path = dir.path().join("out.bin");

        fs::write(&old_path, OLD_TEXT).unwrap();
        fs::write(&diff_path, b"NOT_A_HDIFF_FILE_CONTENTS").unwrap();

        let op = old_path.to_string_lossy().to_string();
        let dp = diff_path.to_string_lossy().to_string();
        let tp = out_path.to_string_lossy().to_string();

        let mut hdiff = HDiff::new(op, dp, tp);
        assert!(!hdiff.apply(), "should fail for invalid diff file");
    }

    #[test]
    fn hdiff_detect_hdiff_magic_bytes() {
        assert!(
            DIFF_V13_TEXT.starts_with(b"HDIFF"),
            "diff data should start with HDIFF magic bytes"
        );
    }

    #[test]
    fn hdiff_nonexistent_source_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        let diff_path = dir.path().join("diff.hdiff");
        let out_path = dir.path().join("out.bin");

        fs::write(&diff_path, DIFF_V13_TEXT).unwrap();

        let op = dir
            .path()
            .join("nonexistent.bin")
            .to_string_lossy()
            .to_string();
        let dp = diff_path.to_string_lossy().to_string();
        let tp = out_path.to_string_lossy().to_string();

        let mut hdiff = HDiff::new(op, dp, tp);
        assert!(!hdiff.apply(), "should fail for nonexistent source file");
    }

    #[test]
    fn hdiff_nonexistent_diff_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("old.bin");
        let out_path = dir.path().join("out.bin");

        fs::write(&old_path, OLD_TEXT).unwrap();

        let op = old_path.to_string_lossy().to_string();
        let dp = dir
            .path()
            .join("nonexistent.hdiff")
            .to_string_lossy()
            .to_string();
        let tp = out_path.to_string_lossy().to_string();

        let mut hdiff = HDiff::new(op, dp, tp);
        assert!(!hdiff.apply(), "should fail for nonexistent diff file");
    }
}
