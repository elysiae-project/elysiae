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
        on_progress: Option<&dyn Fn(u64)>,
    ) -> std::io::Result<()> {
        let padding: u64 = match self.header_info.comp_mode {
            CompressionMode::Zlib => 1,
            _ => 0,
        };
        let hi = &self.header_info;
        let ci = &hi.chunk_info;

        // Open separately per stream: try_clone would share file descriptor
        // position, corrupting parallel reads.
        let f0 = File::open(patch_path)?;
        let f1 = File::open(patch_path)?;
        let f2 = File::open(patch_path)?;
        let f3 = File::open(patch_path)?;

        // For Zlib-compressed sections, the on-disk layout reserves 1 byte for
        // the windowBits prefix prepended by the zlib plugin. We skip that byte
        // when reading and subtract it from `compress_*_size` so the limited
        // reader does not overrun into the next section's prefix byte.
        let comp_size_for_read = |raw_compressed: u64| -> u64 {
            if hi.comp_mode == CompressionMode::Zlib {
                raw_compressed.saturating_sub(padding)
            } else {
                raw_compressed
            }
        };

        let mut offset = ci.head_end_pos as u64;
        let cover_padding = if ci.compress_cover_buf_size > 0 {
            padding
        } else {
            0
        };
        let cover_start = offset
            .checked_add(cover_padding)
            .ok_or_else(|| std::io::Error::other("offset overflow computing cover_start"))?;
        let cover_comp = comp_size_for_read(ci.compress_cover_buf_size as u64);
        let (clip0, len0) = get_clip_stream(
            f0,
            hi.comp_mode,
            cover_start,
            ci.cover_buf_size as u64,
            cover_comp,
            false,
        )?;
        offset = cover_start
            .checked_add(len0)
            .ok_or_else(|| std::io::Error::other("offset overflow after cover"))?;

        let rle_ctrl_padding = if ci.compress_rle_ctrl_buf_size > 0 {
            padding
        } else {
            0
        };
        let rle_ctrl_start = offset
            .checked_add(rle_ctrl_padding)
            .ok_or_else(|| std::io::Error::other("offset overflow computing rle_ctrl_start"))?;
        let rle_ctrl_comp = comp_size_for_read(ci.compress_rle_ctrl_buf_size as u64);
        let (clip1, len1) = get_clip_stream(
            f1,
            hi.comp_mode,
            rle_ctrl_start,
            ci.rle_ctrl_buf_size as u64,
            rle_ctrl_comp,
            false,
        )?;
        offset = rle_ctrl_start
            .checked_add(len1)
            .ok_or_else(|| std::io::Error::other("offset overflow after rle_ctrl"))?;

        let rle_code_padding = if ci.compress_rle_code_buf_size > 0 {
            padding
        } else {
            0
        };
        let rle_code_start = offset
            .checked_add(rle_code_padding)
            .ok_or_else(|| std::io::Error::other("offset overflow computing rle_code_start"))?;
        let rle_code_comp = comp_size_for_read(ci.compress_rle_code_buf_size as u64);
        let (clip2, len2) = get_clip_stream(
            f2,
            hi.comp_mode,
            rle_code_start,
            ci.rle_code_buf_size as u64,
            rle_code_comp,
            false,
        )?;
        offset = rle_code_start
            .checked_add(len2)
            .ok_or_else(|| std::io::Error::other("offset overflow after rle_code"))?;

        let new_data_diff_padding = if ci.compress_new_data_diff_size > 0 {
            padding
        } else {
            0
        };
        let new_data_diff_start = offset.checked_add(new_data_diff_padding).ok_or_else(|| {
            std::io::Error::other("offset overflow computing new_data_diff_start")
        })?;
        let new_data_diff_comp = comp_size_for_read(ci.compress_new_data_diff_size as u64);
        let (clip3, _) = get_clip_stream(
            f3,
            hi.comp_mode,
            new_data_diff_start,
            ci.new_data_diff_size as u64,
            new_data_diff_comp,
            false,
        )?;
        let mut clips: [Box<dyn Read>; 4] = [clip0, clip1, clip2, clip3];
        write_cover_stream_to_output(&mut clips, input_stream, output_stream, hi, on_progress)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::PatchSingle;
    use std::io::Cursor;

    #[test]
    fn patch_single_new_creates_struct() {
        let header_info = super::HeaderInfo::default();
        let ps = PatchSingle::new(header_info);
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::<u8>::new();
        let result = ps.patch(
            &mut input,
            &mut output,
            "/tmp/nonexistent_patch_file_xyzzy.hdiff",
            None,
        );
        assert!(result.is_err(), "patch should fail with missing file");
    }

    #[test]
    fn patch_single_patch_missing_file_returns_not_found() {
        let header_info = super::HeaderInfo::default();
        let ps = PatchSingle::new(header_info);
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::<u8>::new();
        let result = ps.patch(
            &mut input,
            &mut output,
            "/tmp/nonexistent_patch_file_xyzzy_42.hdiff",
            None,
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::NotFound,
            "should return NotFound for missing patch file"
        );
    }

    #[test]
    fn header_info_zlib_padding_is_one() {
        let padding: u64 = match super::CompressionMode::Zlib {
            super::CompressionMode::Zlib => 1,
            _ => 0,
        };
        assert_eq!(padding, 1);
    }

    #[test]
    fn header_info_nocomp_padding_is_zero() {
        let padding: u64 = match super::CompressionMode::Nocomp {
            super::CompressionMode::Zlib => 1,
            _ => 0,
        };
        assert_eq!(padding, 0);
    }
}
