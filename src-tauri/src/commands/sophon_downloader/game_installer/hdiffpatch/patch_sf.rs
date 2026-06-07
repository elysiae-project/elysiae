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
        let mut step_buf = vec![0u8; step_mem_size];
        let mut io_buf = vec![0u8; step_mem_size];
        patch_loop(
            &mut diff,
            input_stream,
            output_stream,
            cover_count,
            new_data_size,
            &mut step_buf,
            &mut io_buf,
        )
    }
}

fn patch_loop(
    mut diff: &mut dyn Read,
    old: &mut dyn SeekableRead,
    out: &mut dyn Write,
    mut cover_count: u64,
    new_data_size: u64,
    step_buf: &mut Vec<u8>,
    io_buf: &mut [u8],
) -> std::io::Result<()> {
    let mut last_old_end = 0u64;
    let mut last_new_end = 0u64;

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
            step_buf.resize(step_end, 0);
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
                copy_n(&mut *diff, out, new_pos - prev_new_end, io_buf)?;
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
            }
        }
    }
    if last_new_end < new_data_size {
        copy_n(&mut *diff, out, new_data_size - last_new_end, io_buf)?;
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
                let to_read = self.lenv.min(available).min(rem);
                if to_read == 0 {
                    self.lenv = 0;
                    continue;
                }
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
                break;
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
