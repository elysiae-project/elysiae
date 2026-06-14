use std::fs::File;
use std::io::{Cursor, Read, SeekFrom, Write};

use super::compression::get_clip_stream;
use super::parser::BinaryExtensions;
use super::{HeaderInfo, SeekableRead};

const MAX_STEP_SIZE: usize = 16 * 1024 * 1024; // 16MB max step size

pub(crate) struct PatchSF {
    header_info: HeaderInfo,
}

impl PatchSF {
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
        let sci = &self.header_info.single_chunk_info;
        let (mut diff, _) = get_clip_stream(
            File::open(patch_path)?,
            self.header_info.comp_mode,
            sci.diff_data_pos as u64,
            sci.uncompressed_size as u64,
            sci.compressed_size as u64,
            false,
        )?;
        if self.header_info.chunk_info.cover_count < 0 {
            return Err(std::io::Error::other("cover_count is negative"));
        }
        if self.header_info.new_data_size < 0 {
            return Err(std::io::Error::other("new_data_size is negative"));
        }
        if self.header_info.step_mem_size <= 0 {
            return Err(std::io::Error::other("step_mem_size is non-positive"));
        }
        let cover_count = self.header_info.chunk_info.cover_count as u64;
        let new_data_size = self.header_info.new_data_size as u64;
        let step_mem_size = (self.header_info.step_mem_size as usize).min(MAX_STEP_SIZE);
        let total_size = step_mem_size * 2;
        let mut work_buf = vec![0u8; total_size];
        let (step_buf, io_buf) = work_buf.split_at_mut(step_mem_size);
        patch_loop(
            &mut diff,
            input_stream,
            output_stream,
            cover_count,
            new_data_size,
            step_buf,
            io_buf,
            on_progress,
        )
    }
}

fn patch_loop(
    mut diff: &mut dyn Read,
    old: &mut dyn SeekableRead,
    out: &mut dyn Write,
    mut cover_count: u64,
    new_data_size: u64,
    step_buf: &mut [u8],
    io_buf: &mut [u8],
    on_progress: Option<&dyn Fn(u64)>,
) -> std::io::Result<()> {
    let mut last_old_end = 0u64;
    let mut last_new_end = 0u64;
    let mut total_written: u64 = 0;

    while cover_count > 0 {
        let buf_cover_size_raw = diff.read_long_7bit()?;
        let buf_rle_size_raw = diff.read_long_7bit()?;
        if buf_cover_size_raw < 0 || buf_rle_size_raw < 0 {
            return Err(std::io::Error::other("negative step size in patch"));
        }
        let buf_cover_size = buf_cover_size_raw as usize;
        let buf_rle_size = buf_rle_size_raw as usize;
        let step_end = buf_cover_size
            .checked_add(buf_rle_size)
            .ok_or_else(|| std::io::Error::other("step size overflow"))?;

        if step_end > MAX_STEP_SIZE {
            return Err(std::io::Error::other(
                "patch step size exceeds maximum allowed",
            ));
        }
        if step_end > step_buf.len() {
            return Err(std::io::Error::other(
                "patch step size exceeds allocated buffer capacity",
            ));
        }
        diff.read_exact(&mut step_buf[..step_end])?;

        let (covers_slice, rle_slice) = step_buf[..step_end].split_at(buf_cover_size);
        let mut covers = Cursor::new(covers_slice);
        let covers_len = covers_slice.len() as u64;
        let mut rle0 = Rle0Decoder::new(rle_slice);

        while covers.position() < covers_len && cover_count > 0 {
            let prev_new_end = last_new_end;
            let (old_pos, new_pos, length) =
                decode_cover(&mut covers, &mut last_old_end, &mut last_new_end)?;
            if new_pos < prev_new_end {
                return Err(std::io::Error::other(
                    "backward or overlapping covers in single-frame patch",
                ));
            }
            if new_pos > prev_new_end {
                let gap = new_pos - prev_new_end;
                copy_n(&mut *diff, out, gap, io_buf)?;
                total_written += gap;
                if let Some(ref cb) = on_progress {
                    cb(total_written);
                }
            }
            cover_count -= 1;

            if length > 0 {
                old.seek(SeekFrom::Start(old_pos))?;
                let mut rem = length;
                while rem > 0 {
                    let take = (io_buf.len() as u64).min(rem) as usize;
                    old.read_exact(&mut io_buf[..take])?;
                    rle0.add(&mut io_buf[..take])?;
                    out.write_all(&io_buf[..take])?;
                    rem -= take as u64;
                }
                total_written += length;
                if let Some(ref cb) = on_progress {
                    cb(total_written);
                }
            }
        }
    }
    if last_new_end < new_data_size {
        let tail = new_data_size - last_new_end;
        copy_n(&mut *diff, out, tail, io_buf)?;
        total_written += tail;
        if let Some(ref cb) = on_progress {
            cb(total_written);
        }
    }
    Ok(())
}

fn decode_cover(
    covers: &mut Cursor<&[u8]>,
    last_old_end: &mut u64,
    last_new_end: &mut u64,
) -> std::io::Result<(u64, u64, u64)> {
    let mut b = [0u8; 1];
    covers.read_exact(&mut b)?;
    let first = b[0];
    let sign = first >> 7;
    let delta_raw = covers.read_long_7bit_tagged(1, first)?;
    if delta_raw < 0 {
        return Err(std::io::Error::other("negative delta in cover header"));
    }
    let delta = delta_raw as u64;
    let old_pos = if sign == 0 {
        last_old_end
            .checked_add(delta)
            .ok_or_else(|| std::io::Error::other("old_pos overflow"))?
    } else {
        last_old_end
            .checked_sub(delta)
            .ok_or_else(|| std::io::Error::other("old_pos underflow"))?
    };
    let new_pos_raw = covers.read_long_7bit()?;
    if new_pos_raw < 0 {
        return Err(std::io::Error::other("negative new_pos gap in cover"));
    }
    let new_pos = last_new_end
        .checked_add(new_pos_raw as u64)
        .ok_or_else(|| std::io::Error::other("new_pos overflow"))?;
    let length_raw = covers.read_long_7bit()?;
    if length_raw < 0 {
        return Err(std::io::Error::other("negative cover length in cover"));
    }
    let length = length_raw as u64;
    *last_old_end = old_pos
        .checked_add(length)
        .ok_or_else(|| std::io::Error::other("old_end overflow"))?;
    *last_new_end = new_pos
        .checked_add(length)
        .ok_or_else(|| std::io::Error::other("new_end overflow"))?;
    Ok((old_pos, new_pos, length))
}

struct Rle0Decoder<'a> {
    buf: &'a [u8],
    pos: usize,
    len0: usize,
    lenv: usize,
    need_decode0: bool,
}

impl<'a> Rle0Decoder<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            pos: 0,
            len0: 0,
            lenv: 0,
            need_decode0: true,
        }
    }

    fn add(&mut self, data: &mut [u8]) -> std::io::Result<()> {
        let mut dp = 0usize;
        let mut rem = data.len();
        while rem > 0 {
            if self.len0 > 0 {
                let take = self.len0.min(rem);
                self.len0 -= take;
                dp += take;
                rem -= take;
            } else if self.lenv > 0 {
                let available = self.buf.len().saturating_sub(self.pos);
                if self.lenv > available {
                    return Err(std::io::Error::other(
                        "RLE0 diff data exceeds remaining code buffer",
                    ));
                }
                let to_read = self.lenv.min(rem);
                let src = &self.buf[self.pos..self.pos + to_read];
                for i in 0..to_read {
                    data[dp + i] = data[dp + i].wrapping_add(src[i]);
                }
                self.pos += to_read;
                self.lenv -= to_read;
                dp += to_read;
                rem -= to_read;
            } else if self.need_decode0 {
                self.need_decode0 = false;
                match rle_varint(self.buf, &mut self.pos) {
                    Some(v) => self.len0 = v,
                    None => {
                        return Err(std::io::Error::other("truncated RLE varint in RLE0 (len0)"));
                    }
                }
            } else {
                self.need_decode0 = true;
                match rle_varint(self.buf, &mut self.pos) {
                    Some(v) => self.lenv = v,
                    None => {
                        return Err(std::io::Error::other("truncated RLE varint in RLE0 (lenv)"));
                    }
                }
            }
        }
        Ok(())
    }
}

fn rle_varint(buf: &[u8], pos: &mut usize) -> Option<usize> {
    if *pos >= buf.len() {
        return None;
    }
    let first = buf[*pos];
    *pos += 1;
    let mut val = (first & 0x7F) as u64;
    if (first & 0x80) != 0 {
        loop {
            if val >= (u64::MAX >> 7) {
                return None;
            }
            if *pos >= buf.len() {
                return None;
            }
            let b = buf[*pos];
            *pos += 1;
            val = (val << 7) | (b & 0x7F) as u64;
            if (b & 0x80) == 0 {
                break;
            }
        }
    }
    if val > usize::MAX as u64 {
        return None;
    }
    Some(val as usize)
}

fn copy_n(
    src: &mut dyn Read,
    dst: &mut dyn Write,
    mut n: u64,
    buf: &mut [u8],
) -> std::io::Result<()> {
    while n > 0 {
        let take = (buf.len() as u64).min(n) as usize;
        src.read_exact(&mut buf[..take])?;
        dst.write_all(&buf[..take])?;
        n -= take as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{CompressionMode, DiffChunkInfo, DiffSingleChunkInfo};
    use super::*;

    #[test]
    fn rle_varint_empty_buffer_returns_none() {
        let mut pos = 0;
        assert_eq!(rle_varint(&[], &mut pos), None);
    }

    #[test]
    fn rle_varint_single_byte_ok() {
        let buf = [0b00000010]; // value = 2
        let mut pos = 0;
        assert_eq!(rle_varint(&buf, &mut pos), Some(2));
        assert_eq!(pos, 1);
    }

    #[test]
    fn rle_varint_multi_byte_ok() {
        // The encoding is big-endian 7-bit groups with continuation in MSB.
        // Value 256 = 0x100 = (2 << 7) | 0
        // First byte: 2 with continuation = 0x82
        // Second byte: 0 with no continuation = 0x00
        let buf = [0x82, 0x00];
        let mut pos = 0;
        assert_eq!(rle_varint(&buf, &mut pos), Some(256));
        assert_eq!(pos, 2);
    }

    #[test]
    fn rle_varint_truncated_multi_byte_returns_none() {
        // Truncated multi-byte varint (continuation set on first byte but no second
        // byte)
        let buf = [0x80];
        let mut pos = 0;
        assert_eq!(rle_varint(&buf, &mut pos), None);
    }

    #[test]
    fn rle0decoder_skip_all_bytes() {
        // RLE stream: [3] -> skip 3 bytes
        let rle_buf = [3u8];
        let mut rle0 = Rle0Decoder::new(&rle_buf);
        let mut data = [0xAAu8, 0xBB, 0xCC];
        rle0.add(&mut data).unwrap();
        // Should skip all three bytes without change
        assert_eq!(data, [0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn rle0decoder_xor_some_bytes() {
        // RLE stream: skip 1 byte, then XOR 1 byte with 0x42
        // varints: len0=1, lenv=1
        // diff data after varints: 0x42
        let rle_buf = [1u8, 1u8, 0x42u8];
        let mut rle0 = Rle0Decoder::new(&rle_buf);
        let mut data = [0x00u8, 0x00u8];
        rle0.add(&mut data).unwrap();
        // First byte skipped, second byte XORed with 0x42
        assert_eq!(data, [0x00, 0x42]);
    }

    #[test]
    fn rle0decoder_truncated_varint_fails() {
        // Truncated rle0 stream: just continuation marker with no follow-up
        let rle_buf = [0x80]; // continuation bit set, but no next byte
        let mut rle0 = Rle0Decoder::new(&rle_buf);
        let mut data = [0u8; 1];
        let result = rle0.add(&mut data);
        assert!(result.is_err());
    }

    #[test]
    fn rle_varint_0x3fff_ok() {
        // 0x3FFF = 16383 = (127 << 7) | 127
        // First byte: 127 with continuation = 0xFF
        // Second byte: 127 without continuation = 0x7F
        let buf = [0xFF, 0x7F];
        let mut pos = 0;
        let result = rle_varint(&buf, &mut pos);
        assert_eq!(result, Some(0x3FFF));
        assert_eq!(pos, 2);
    }

    #[test]
    fn rle_varint_single_byte_max_value() {
        // Single byte with no continuation: max value is 0x7F = 127
        let buf = [0x7F];
        let mut pos = 0;
        assert_eq!(rle_varint(&buf, &mut pos), Some(127));
        assert_eq!(pos, 1);
    }

    #[test]
    fn rle_varint_single_byte_with_continuation() {
        // Single byte with continuation set: 0x81 = value 1 with more bytes
        // Next byte: 0x00 with no continuation = end
        // Value = (1 << 7) | 0 = 128
        let buf = [0x81, 0x00];
        let mut pos = 0;
        assert_eq!(rle_varint(&buf, &mut pos), Some(128));
        assert_eq!(pos, 2);
    }

    #[test]
    fn rle_varint_value_128_two_byte_encoding() {
        // Value 128 = (1 << 7) | 0
        // Needs 2 bytes because 128 requires 8 bits but only 7 per byte
        // First byte: (1 & 0x7F) | 0x80 = 0x81 (continuation set, value 1)
        // Second byte: (0 & 0x7F) = 0x00 (no continuation, value 0)
        // Combined: (1 << 7) | 0 = 128
        let buf = [0x81, 0x00];
        let mut pos = 0;
        assert_eq!(rle_varint(&buf, &mut pos), Some(128));
    }

    #[test]
    fn rle0decoder_alternating_skip_xor() {
        // Pattern: skip 1, xor 1, skip 1, xor 1
        // RLE stream: skip=1 (varint), xor_val=1 (varint), then 0xAA
        //            skip=1 (varint), xor_val=1 (varint), then 0xBB
        let rle_buf = [1u8, 1u8, 0xAAu8, 1u8, 1u8, 0xBBu8];
        let mut rle0 = Rle0Decoder::new(&rle_buf);
        let mut data = [0x00u8, 0x00u8, 0x00u8, 0x00u8];
        rle0.add(&mut data).unwrap();
        // Skip 1 (keep 0x00), XOR 1 with 0xAA -> 0xAA
        // Skip 1 (keep 0x00), XOR 1 with 0xBB -> 0xBB
        assert_eq!(data, [0x00, 0xAA, 0x00, 0xBB]);
    }

    #[test]
    fn rle0decoder_large_skip() {
        // Skip 10 bytes, then XOR 2 bytes
        // RLE stream: skip=10, len=2, xor_val=0x11, xor_val=0x22
        // Skip 10 bytes means copy 10 bytes unchanged, then apply XOR
        let mut rle_buf = Vec::new();
        // skip=10 encoded as rle varint
        rle_buf.push(10u8);
        // len=2
        rle_buf.push(2u8);
        // XOR values
        rle_buf.push(0x11u8);
        rle_buf.push(0x22u8);

        let mut rle0 = Rle0Decoder::new(&rle_buf);
        let mut data = [0x00u8; 12];
        rle0.add(&mut data).unwrap();
        // First 10 bytes unchanged (0x00), last 2 XORed
        assert_eq!(data[..10], [0x00u8; 10]);
        assert_eq!(data[10], 0x11);
        assert_eq!(data[11], 0x22);
    }

    #[test]
    fn patch_sf_new_creates_struct() {
        let header = HeaderInfo::default();
        let patcher = PatchSF::new(header);
        assert_eq!(patcher.header_info.comp_mode, CompressionMode::Nocomp);
    }

    #[test]
    fn patch_sf_patch_missing_file_returns_not_found() {
        let header = HeaderInfo {
            single_chunk_info: DiffSingleChunkInfo {
                diff_data_pos: 0,
                uncompressed_size: 1,
                compressed_size: 0,
            },
            comp_mode: CompressionMode::Nocomp,
            chunk_info: DiffChunkInfo {
                cover_count: 1,
                ..Default::default()
            },
            new_data_size: 1,
            step_mem_size: 1,
            ..Default::default()
        };
        let patcher = PatchSF::new(header);
        let mut input = Cursor::new(Vec::new());
        let mut output = Cursor::new(Vec::new());
        let result = patcher.patch(
            &mut input,
            &mut output,
            "/nonexistent/patch_file.hdiff",
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn patch_sf_patch_negative_cover_count_fails() {
        let dir = tempfile::tempdir().unwrap();
        let patch_path = dir.path().join("patch.hdiff");
        std::fs::write(&patch_path, b"").unwrap();

        let header = HeaderInfo {
            single_chunk_info: DiffSingleChunkInfo {
                diff_data_pos: 0,
                uncompressed_size: 0,
                compressed_size: 0,
            },
            comp_mode: CompressionMode::Nocomp,
            chunk_info: DiffChunkInfo {
                cover_count: -1,
                ..Default::default()
            },
            new_data_size: 1,
            step_mem_size: 1,
            ..Default::default()
        };
        let patcher = PatchSF::new(header);
        let mut input = Cursor::new(Vec::new());
        let mut output = Cursor::new(Vec::new());
        let result = patcher.patch(&mut input, &mut output, patch_path.to_str().unwrap(), None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("cover_count is negative"), "msg={msg}");
    }

    #[test]
    fn patch_sf_patch_negative_new_data_size_fails() {
        let dir = tempfile::tempdir().unwrap();
        let patch_path = dir.path().join("patch.hdiff");
        std::fs::write(&patch_path, b"").unwrap();

        let header = HeaderInfo {
            single_chunk_info: DiffSingleChunkInfo {
                diff_data_pos: 0,
                uncompressed_size: 0,
                compressed_size: 0,
            },
            comp_mode: CompressionMode::Nocomp,
            chunk_info: DiffChunkInfo {
                cover_count: 0,
                ..Default::default()
            },
            new_data_size: -1,
            step_mem_size: 1,
            ..Default::default()
        };
        let patcher = PatchSF::new(header);
        let mut input = Cursor::new(Vec::new());
        let mut output = Cursor::new(Vec::new());
        let result = patcher.patch(&mut input, &mut output, patch_path.to_str().unwrap(), None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("new_data_size is negative"), "msg={msg}");
    }

    #[test]
    fn patch_sf_patch_non_positive_step_mem_size_fails() {
        let dir = tempfile::tempdir().unwrap();
        let patch_path = dir.path().join("patch.hdiff");
        std::fs::write(&patch_path, b"").unwrap();

        let header = HeaderInfo {
            single_chunk_info: DiffSingleChunkInfo {
                diff_data_pos: 0,
                uncompressed_size: 0,
                compressed_size: 0,
            },
            comp_mode: CompressionMode::Nocomp,
            chunk_info: DiffChunkInfo {
                cover_count: 0,
                ..Default::default()
            },
            new_data_size: 0,
            step_mem_size: 0,
            ..Default::default()
        };
        let patcher = PatchSF::new(header);
        let mut input = Cursor::new(Vec::new());
        let mut output = Cursor::new(Vec::new());
        let result = patcher.patch(&mut input, &mut output, patch_path.to_str().unwrap(), None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("step_mem_size is non-positive"), "msg={msg}");
    }
}
