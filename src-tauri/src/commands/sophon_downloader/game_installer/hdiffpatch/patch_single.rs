use std::fs::File;
use std::io::{Read, Write};

use super::compression::get_clip_stream;
use super::patch_core::write_cover_stream_to_output;
use super::{CompressionMode, HeaderInfo, SeekableRead};

pub(crate) struct PatchSingle {
    header_info: HeaderInfo,
}

impl PatchSingle {
    pub fn new(header_info: HeaderInfo) -> Self {
        Self { header_info }
    }

    pub fn patch(
        &self,
        input_stream: &mut dyn SeekableRead,
        output_stream: &mut dyn Write,
        patch_path: &str,
    ) -> std::io::Result<()> {
        let padding: u64 = match self.header_info.comp_mode {
            CompressionMode::Zlib => 1,
            _ => 0,
        };
        let hi = &self.header_info;
        let ci = &hi.chunk_info;

        let f0 = File::open(patch_path)?;
        let f1 = File::open(patch_path)?;
        let f2 = File::open(patch_path)?;
        let f3 = File::open(patch_path)?;

        let mut offset = ci.head_end_pos as u64;
        let cover_padding = if ci.compress_cover_buf_size > 1 {
            padding
        } else {
            0
        };
        let cover_start = offset.wrapping_add(cover_padding);
        let (clip0, len0) = get_clip_stream(
            f0,
            hi.comp_mode,
            cover_start,
            ci.cover_buf_size as u64,
            ci.compress_cover_buf_size as u64,
            false,
        )?;
        offset = cover_start.wrapping_add(len0);

        let rle_ctrl_padding = if ci.compress_rle_ctrl_buf_size > 0 {
            padding
        } else {
            0
        };
        let rle_ctrl_start = offset.wrapping_add(rle_ctrl_padding);
        let (clip1, len1) = get_clip_stream(
            f1,
            hi.comp_mode,
            rle_ctrl_start,
            ci.rle_ctrl_buf_size as u64,
            ci.compress_rle_ctrl_buf_size as u64,
            false,
        )?;
        offset = rle_ctrl_start.wrapping_add(len1);

        let rle_code_padding = if ci.compress_rle_code_buf_size > 0 {
            padding
        } else {
            0
        };
        let rle_code_start = offset.wrapping_add(rle_code_padding);
        let (clip2, len2) = get_clip_stream(
            f2,
            hi.comp_mode,
            rle_code_start,
            ci.rle_code_buf_size as u64,
            ci.compress_rle_code_buf_size as u64,
            false,
        )?;
        offset = rle_code_start.wrapping_add(len2);

        let new_data_diff_padding = if ci.compress_new_data_diff_size > 0 {
            padding
        } else {
            0
        };
        let new_data_diff_start = offset.wrapping_add(new_data_diff_padding);
        let (clip3, _) = get_clip_stream(
            f3,
            hi.comp_mode,
            new_data_diff_start,
            ci.new_data_diff_size as u64,
            ci.compress_new_data_diff_size as u64,
            false,
        )?;
        let mut clips: [Box<dyn Read>; 4] = [clip0, clip1, clip2, clip3];
        write_cover_stream_to_output(&mut clips, input_stream, output_stream, hi)?;
        Ok(())
    }
}
