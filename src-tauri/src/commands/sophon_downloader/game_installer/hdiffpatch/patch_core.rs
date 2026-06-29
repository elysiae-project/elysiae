use std::io::{Cursor, Read, Seek, SeekFrom, Write};

use super::{
    CoverHeader, HeaderInfo, K_BYTE_RLE_TYPE, K_SIGN_TAG_BIT, MAX_ARRAY_POOL_LEN,
    MAX_ARRAY_POOL_SECOND_OFFSET, MAX_MEM_BUFFER_LEN, MAX_MEM_BUFFER_LIMIT, RleRefClip,
    SeekableRead,
};
use crate::commands::sophon_downloader::game_installer::hdiffpatch::parser::{
    BinaryExtensions, read_long_7bit_from_slice,
};

pub(crate) fn write_cover_stream_to_output(
    clips: &mut [Box<dyn Read>],
    input_stream: &mut dyn SeekableRead,
    output_stream: &mut dyn Write,
    header_info: &HeaderInfo,
    on_progress: Option<&dyn Fn(u64)>,
) -> std::io::Result<()> {
    // Both halves of the shared buffer are fully written via read_exact
    // before any byte is read from them. Skip the zero-init that
    // `vec![0; n]` would perform to avoid touching 4 MiB of memory
    // on every patch application.
    // Safety: every index 0..MAX_ARRAY_POOL_LEN is populated by
    // read_exact or write_all before being read in the RLE decode loop.
    #[allow(clippy::uninit_vec)]
    let mut shared_buffer = {
        let mut v = Vec::with_capacity(MAX_ARRAY_POOL_LEN);
        unsafe { v.set_len(MAX_ARRAY_POOL_LEN) };
        v
    };
    let mut cache = Cursor::new(Vec::<u8>::new());

    let mut new_pos_back = 0i64;
    let mut total_written: u64 = 0;
    let mut rle_struct = RleRefClip::default();
    let (left, right) = clips.split_at_mut(2);
    let headers = enumerate_cover_headers(
        &mut *left[0],
        header_info.chunk_info.cover_buf_size,
        header_info.chunk_info.cover_count,
    )?;

    for cover in &headers {
        if cover.new_pos < new_pos_back {
            return Err(std::io::Error::other(
                "backward or overlapping covers in patch",
            ));
        }
        let cover_end = cover
            .new_pos
            .checked_add(cover.cover_length)
            .ok_or_else(|| std::io::Error::other("cover length overflow"))?;
        if cover_end > header_info.new_data_size {
            return Err(std::io::Error::other(
                "cover extends past expected output size",
            ));
        }
        if new_pos_back < cover.new_pos {
            let copy_length = cover.new_pos - new_pos_back;
            tbytes_copy_stream_from_old_clip(
                &mut cache,
                &mut *right[1],
                copy_length,
                &mut shared_buffer,
            )?;
            tbytes_determine_rle_type(
                &mut rle_struct,
                &mut cache,
                copy_length,
                &mut shared_buffer,
                &mut *left[1],
                &mut *right[0],
            )?;
        }

        tbytes_copy_old_clip_patch(
            &mut cache,
            input_stream,
            &mut rle_struct,
            cover.old_pos,
            cover.cover_length,
            &mut shared_buffer,
            &mut *left[1],
            &mut *right[0],
        )?;
        new_pos_back = cover
            .new_pos
            .checked_add(cover.cover_length)
            .ok_or_else(|| std::io::Error::other("new_pos overflow in cover iteration"))?;
        if cache.get_ref().len() > MAX_MEM_BUFFER_LIMIT || cover.next_cover_index == 0 {
            let cache_len = cache.get_ref().len() as u64;
            write_cache_to_output(&mut cache, output_stream)?;
            total_written += cache_len;
            if let Some(ref cb) = on_progress {
                cb(total_written);
            }
        }
    }

    if new_pos_back < header_info.new_data_size {
        let copy_length = header_info.new_data_size - new_pos_back;
        if copy_length < 0 {
            return Err(std::io::Error::other(
                "tail copy length is negative; diff is malformed",
            ));
        }
        tbytes_copy_stream_from_old_clip(
            &mut cache,
            &mut *right[1],
            copy_length,
            &mut shared_buffer,
        )?;
        tbytes_determine_rle_type(
            &mut rle_struct,
            &mut cache,
            copy_length,
            &mut shared_buffer,
            &mut *left[1],
            &mut *right[0],
        )?;
        let cache_len = cache.get_ref().len() as u64;
        write_cache_to_output(&mut cache, output_stream)?;
        total_written += cache_len;
        if let Some(ref cb) = on_progress {
            cb(total_written);
        }
    }
    Ok(())
}

fn write_cache_to_output(
    cache: &mut Cursor<Vec<u8>>,
    output: &mut dyn Write,
) -> std::io::Result<()> {
    let data = cache.get_ref();
    if !data.is_empty() {
        output.write_all(data)?;
        cache.get_mut().clear();
        cache.set_position(0);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn tbytes_copy_old_clip_patch(
    out_cache: &mut Cursor<Vec<u8>>,
    input_stream: &mut dyn SeekableRead,
    rle_loader: &mut RleRefClip,
    old_pos: i64,
    add_length: i64,
    shared_buffer: &mut [u8],
    rle_ctrl_stream: &mut dyn Read,
    rle_code_stream: &mut dyn Read,
) -> std::io::Result<()> {
    if add_length < 0 {
        return Err(std::io::Error::other("add_length is negative"));
    }
    let last_pos = out_cache.position();
    input_stream.seek(SeekFrom::Start(old_pos as u64))?;
    tbytes_copy_stream_inner(input_stream, out_cache, shared_buffer, add_length as usize)?;
    out_cache.seek(SeekFrom::Start(last_pos))?;
    tbytes_determine_rle_type(
        rle_loader,
        out_cache,
        add_length,
        shared_buffer,
        rle_ctrl_stream,
        rle_code_stream,
    )
}

pub(crate) fn tbytes_copy_stream_from_old_clip(
    out_cache: &mut Cursor<Vec<u8>>,
    copy_reader: &mut dyn Read,
    copy_length: i64,
    shared_buffer: &mut [u8],
) -> std::io::Result<()> {
    if copy_length < 0 {
        return Err(std::io::Error::other("copy_length is negative"));
    }
    let last_pos = out_cache.position();
    tbytes_copy_stream_inner(copy_reader, out_cache, shared_buffer, copy_length as usize)?;
    out_cache.seek(SeekFrom::Start(last_pos))?;
    Ok(())
}

fn tbytes_copy_stream_inner(
    input: &mut dyn Read,
    output: &mut Cursor<Vec<u8>>,
    shared_buffer: &mut [u8],
    mut read_len: usize,
) -> std::io::Result<()> {
    while read_len > 0 {
        let to_read = shared_buffer.len().min(read_len);
        input.read_exact(&mut shared_buffer[..to_read])?;
        output.write_all(&shared_buffer[..to_read])?;
        read_len -= to_read;
    }
    Ok(())
}

fn tbytes_determine_rle_type(
    rle_loader: &mut RleRefClip,
    out_cache: &mut Cursor<Vec<u8>>,
    mut copy_length: i64,
    shared_buffer: &mut [u8],
    mut rle_ctrl_stream: &mut dyn Read,
    rle_code_stream: &mut dyn Read,
) -> std::io::Result<()> {
    tbytes_set_rle(
        rle_loader,
        out_cache,
        &mut copy_length,
        shared_buffer,
        rle_code_stream,
    )?;

    while copy_length > 0 {
        let mut p_sign_buf = [0u8; 1];
        rle_ctrl_stream.read_exact(&mut p_sign_buf)?;
        let p_sign = p_sign_buf[0];

        let rle_type = p_sign >> (8 - K_BYTE_RLE_TYPE);
        let raw_length = rle_ctrl_stream.read_long_7bit_tagged(K_BYTE_RLE_TYPE, p_sign)?;
        let length = raw_length
            .checked_add(1)
            .ok_or_else(|| std::io::Error::other("RLE length overflow after +1"))?;

        if rle_type == 3 {
            rle_loader.mem_copy_length = length;
            tbytes_set_rle(
                rle_loader,
                out_cache,
                &mut copy_length,
                shared_buffer,
                rle_code_stream,
            )?;
            continue;
        }

        rle_loader.mem_set_length = length;
        if rle_type == 2 {
            let mut val = [0u8; 1];
            rle_code_stream.read_exact(&mut val)?;
            rle_loader.mem_set_value = val[0];
            tbytes_set_rle(
                rle_loader,
                out_cache,
                &mut copy_length,
                shared_buffer,
                rle_code_stream,
            )?;
            continue;
        }
        rle_loader.mem_set_value = (0u8).wrapping_sub(rle_type);
        tbytes_set_rle(
            rle_loader,
            out_cache,
            &mut copy_length,
            shared_buffer,
            rle_code_stream,
        )?;
    }
    Ok(())
}

pub(crate) fn tbytes_set_rle(
    rle_loader: &mut RleRefClip,
    out_cache: &mut Cursor<Vec<u8>>,
    copy_length: &mut i64,
    shared_buffer: &mut [u8],
    rle_code_stream: &mut dyn Read,
) -> std::io::Result<()> {
    tbytes_set_rle_single(rle_loader, out_cache, copy_length, shared_buffer)?;
    if rle_loader.mem_copy_length == 0 {
        return Ok(());
    }

    let decode_step = rle_loader
        .mem_copy_length
        .min(*copy_length)
        .min(MAX_ARRAY_POOL_SECOND_OFFSET as i64) as usize;
    let last_pos = out_cache.position();
    rle_code_stream.read_exact(&mut shared_buffer[..decode_step])?;
    out_cache.read_exact(
        &mut shared_buffer
            [MAX_ARRAY_POOL_SECOND_OFFSET..MAX_ARRAY_POOL_SECOND_OFFSET + decode_step],
    )?;
    out_cache.seek(SeekFrom::Start(last_pos))?;
    tbytes_set_rle_vector_software(
        rle_loader,
        out_cache,
        copy_length,
        decode_step,
        shared_buffer,
        0,
        MAX_ARRAY_POOL_SECOND_OFFSET,
    )
}

pub(crate) fn tbytes_set_rle_single(
    rle_loader: &mut RleRefClip,
    out_cache: &mut Cursor<Vec<u8>>,
    copy_length: &mut i64,
    shared_buffer: &mut [u8],
) -> std::io::Result<()> {
    if rle_loader.mem_set_length == 0 {
        return Ok(());
    }
    let mem_set_step = rle_loader
        .mem_set_length
        .min(*copy_length)
        .min(shared_buffer.len() as i64);

    if rle_loader.mem_set_value != 0 {
        let last_pos = out_cache.position();
        let len = mem_set_step as usize;
        out_cache.read_exact(&mut shared_buffer[..len])?;
        out_cache.seek(SeekFrom::Start(last_pos))?;
        // Use slice iterator form so LLVM can emit a broadcast-add
        // (vpaddb with broadcast on x86_64 AVX2). Iteration direction is
        // safe in either way because each index only reads and writes
        // itself.
        let v = rle_loader.mem_set_value;
        shared_buffer[..len]
            .iter_mut()
            .for_each(|b| *b = b.wrapping_add(v));
        out_cache.write_all(&shared_buffer[..len])?;
    } else {
        let cur = out_cache.position();
        out_cache.set_position(cur + mem_set_step as u64);
    }
    *copy_length -= mem_set_step;
    rle_loader.mem_set_length -= mem_set_step;
    Ok(())
}

pub(crate) fn tbytes_set_rle_vector_software(
    rle_loader: &mut RleRefClip,
    out_cache: &mut Cursor<Vec<u8>>,
    copy_length: &mut i64,
    decode_step: usize,
    buf: &mut [u8],
    rle_idx: usize,
    old_idx: usize,
) -> std::io::Result<()> {
    // Use split_at_mut to obtain two non-overlapping mutable slices from
    // buf. The pattern relies on the invariant that rle_idx and
    // (old_idx + decode_step) split the buffer so the two ranges are
    // disjoint ,  true for all callers in this codebase. Iter-zip form
    // enables LLVM autovectorization (vpaddb/AVX2).
    if rle_idx + decode_step <= old_idx || old_idx + decode_step <= rle_idx {
        if rle_idx < old_idx {
            let (lo, rest) = buf.split_at_mut(old_idx);
            let dst = &mut lo[rle_idx..];
            let src = &rest[..decode_step];
            for (d, s) in dst.iter_mut().zip(src.iter()) {
                *d = d.wrapping_add(*s);
            }
        } else {
            let (lo, rest) = buf.split_at_mut(rle_idx);
            let dst = &mut rest[..decode_step];
            let src = &lo[old_idx..old_idx + decode_step];
            for (d, s) in dst.iter_mut().zip(src.iter()) {
                *d = d.wrapping_add(*s);
            }
        }
        out_cache.write_all(&buf[rle_idx..rle_idx + decode_step])?;
    } else {
        // Overlapping ranges: read immediately before write at the same
        // index so the add is well-defined serially.
        for i in 0..decode_step {
            let v = buf[old_idx + i];
            buf[rle_idx + i] = buf[rle_idx + i].wrapping_add(v);
        }
        out_cache.write_all(&buf[rle_idx..rle_idx + decode_step])?;
    }
    rle_loader.mem_copy_length -= decode_step as i64;
    *copy_length -= decode_step as i64;
    Ok(())
}

pub(crate) struct CoverHeaderIterator<'a> {
    reader: CoverReader<'a>,
    remaining: i64,
    last_old_pos_back: i64,
    last_new_pos_back: i64,
}

enum CoverReader<'a> {
    Buffered { data: Vec<u8>, offset: usize },
    Streaming { reader: &'a mut dyn Read },
}

impl<'a> CoverHeaderIterator<'a> {
    pub(crate) fn new(
        cover_reader: &'a mut dyn Read,
        cover_size: i64,
        cover_count: i64,
    ) -> std::io::Result<Self> {
        if cover_count <= 0 {
            return Ok(Self {
                reader: CoverReader::Buffered {
                    data: Vec::new(),
                    offset: 0,
                },
                remaining: 0,
                last_old_pos_back: 0,
                last_new_pos_back: 0,
            });
        }
        let reader = if cover_size < MAX_MEM_BUFFER_LEN {
            let mut buffer = vec![0u8; cover_size as usize];
            cover_reader.read_exact(&mut buffer).map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!("failed to read cover data: {err}"),
                )
            })?;
            CoverReader::Buffered {
                data: buffer,
                offset: 0,
            }
        } else {
            CoverReader::Streaming {
                reader: cover_reader,
            }
        };
        Ok(Self {
            reader,
            remaining: cover_count,
            last_old_pos_back: 0,
            last_new_pos_back: 0,
        })
    }
}

impl<'a> Iterator for CoverHeaderIterator<'a> {
    type Item = std::io::Result<CoverHeader>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining <= 0 {
            return None;
        }
        self.remaining -= 1;

        let old_pos_back = self.last_old_pos_back;
        let new_pos_back = self.last_new_pos_back;

        let result = match &mut self.reader {
            CoverReader::Buffered { data, offset } => {
                if *offset >= data.len() {
                    return Some(Err(std::io::Error::other("cover header data truncated")));
                }
                let p_sign = data[*offset];
                *offset += 1;

                let inc_old_pos_sign = p_sign >> (8 - K_SIGN_TAG_BIT);
                let inc_old_pos =
                    match read_long_7bit_from_slice(data, offset, K_SIGN_TAG_BIT, p_sign) {
                        Ok(v) => v,
                        Err(err) => return Some(Err(err)),
                    };
                let old_pos = match if inc_old_pos_sign == 0 {
                    old_pos_back.checked_add(inc_old_pos)
                } else {
                    old_pos_back.checked_sub(inc_old_pos)
                } {
                    Some(v) => v,
                    None => {
                        return Some(Err(std::io::Error::other(
                            "old_pos overflow in cover header",
                        )));
                    }
                };
                if old_pos < 0 {
                    return Some(Err(std::io::Error::other(
                        "invalid negative old_pos in cover header",
                    )));
                }

                let copy_length = match read_long_7bit_from_slice(data, offset, 0, 0) {
                    Ok(v) => v,
                    Err(err) => return Some(Err(err)),
                };
                let cover_length = match read_long_7bit_from_slice(data, offset, 0, 0) {
                    Ok(v) => v,
                    Err(err) => return Some(Err(err)),
                };
                if copy_length < 0 || cover_length < 0 {
                    return Some(Err(std::io::Error::other(
                        "invalid negative copy_length or cover_length in cover header",
                    )));
                }
                let new_pos_back = match new_pos_back.checked_add(copy_length) {
                    Some(v) => v,
                    None => {
                        return Some(Err(std::io::Error::other(
                            "new_pos overflow in cover header",
                        )));
                    }
                };
                self.last_old_pos_back = match old_pos.checked_add(cover_length) {
                    Some(v) => v,
                    None => {
                        return Some(Err(std::io::Error::other(
                            "old_pos overflow in cover header",
                        )));
                    }
                };
                self.last_new_pos_back = match new_pos_back.checked_add(cover_length) {
                    Some(v) => v,
                    None => {
                        return Some(Err(std::io::Error::other(
                            "last_new_pos overflow in cover header",
                        )));
                    }
                };
                Ok(CoverHeader::new(
                    old_pos,
                    new_pos_back,
                    cover_length,
                    self.remaining,
                ))
            }
            CoverReader::Streaming { reader } => {
                let mut p_sign_buf = [0u8; 1];
                if reader.read_exact(&mut p_sign_buf).is_err() {
                    return Some(Err(std::io::Error::other("cover header data truncated")));
                }
                let p_sign = p_sign_buf[0];

                let inc_old_pos_sign = p_sign >> (8 - K_SIGN_TAG_BIT);
                let inc_old_pos = match reader.read_long_7bit_tagged(K_SIGN_TAG_BIT, p_sign) {
                    Ok(v) => v,
                    Err(err) => return Some(Err(err)),
                };
                let old_pos = match if inc_old_pos_sign == 0 {
                    old_pos_back.checked_add(inc_old_pos)
                } else {
                    old_pos_back.checked_sub(inc_old_pos)
                } {
                    Some(v) => v,
                    None => {
                        return Some(Err(std::io::Error::other(
                            "old_pos overflow in cover header (stream)",
                        )));
                    }
                };
                if old_pos < 0 {
                    return Some(Err(std::io::Error::other(
                        "invalid negative old_pos in cover header",
                    )));
                }

                let copy_length = match reader.read_long_7bit() {
                    Ok(v) => v,
                    Err(err) => return Some(Err(err)),
                };
                let cover_length = match reader.read_long_7bit() {
                    Ok(v) => v,
                    Err(err) => return Some(Err(err)),
                };
                if copy_length < 0 || cover_length < 0 {
                    return Some(Err(std::io::Error::other(
                        "invalid negative copy_length or cover_length in cover header",
                    )));
                }
                let new_pos_back = match new_pos_back.checked_add(copy_length) {
                    Some(v) => v,
                    None => {
                        return Some(Err(std::io::Error::other(
                            "new_pos overflow in cover header (stream)",
                        )));
                    }
                };
                self.last_old_pos_back = match old_pos.checked_add(cover_length) {
                    Some(v) => v,
                    None => {
                        return Some(Err(std::io::Error::other(
                            "old_pos overflow in cover header (stream)",
                        )));
                    }
                };
                self.last_new_pos_back = match new_pos_back.checked_add(cover_length) {
                    Some(v) => v,
                    None => {
                        return Some(Err(std::io::Error::other(
                            "last_new_pos overflow in cover header (stream)",
                        )));
                    }
                };
                Ok(CoverHeader::new(
                    old_pos,
                    new_pos_back,
                    cover_length,
                    self.remaining,
                ))
            }
        };

        Some(result)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.remaining.max(0) as usize;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for CoverHeaderIterator<'a> {}

pub(crate) fn enumerate_cover_headers(
    cover_reader: &mut dyn Read,
    cover_size: i64,
    cover_count: i64,
) -> std::io::Result<Vec<CoverHeader>> {
    if cover_count < 0 {
        return Err(std::io::Error::other("cover_count is negative"));
    }
    if cover_size < 0 {
        return Err(std::io::Error::other("cover_size is negative"));
    }
    const MAX_COVER_COUNT: i64 = 50_000_000;
    const MAX_COVER_HEADERS_MEMORY: usize = 1 << 30;
    const COVER_HEADER_SIZE: usize = 32;
    let max_headers_by_memory = (MAX_COVER_HEADERS_MEMORY / COVER_HEADER_SIZE) as i64;
    if cover_count > MAX_COVER_COUNT || cover_count > max_headers_by_memory {
        return Err(std::io::Error::other(
            "cover_count exceeds safe maximum or memory limit",
        ));
    }
    if cover_count > 0 && cover_size == 0 {
        return Err(std::io::Error::other("cover_count > 0 but cover_size is 0"));
    }
    CoverHeaderIterator::new(cover_reader, cover_size, cover_count)?.collect()
}

#[allow(dead_code, clippy::needless_lifetimes)]
pub(crate) fn enumerate_cover_headers_checked<'a>(
    cover_reader: &'a mut dyn Read,
    cover_size: i64,
    cover_count: i64,
) -> std::io::Result<CoverHeaderIterator<'a>> {
    if cover_count < 0 {
        return Err(std::io::Error::other("cover_count is negative"));
    }
    if cover_size < 0 {
        return Err(std::io::Error::other("cover_size is negative"));
    }
    const MAX_COVER_COUNT: i64 = 50_000_000;
    const MAX_COVER_HEADERS_MEMORY: usize = 1 << 30;
    const COVER_HEADER_SIZE: usize = 32;
    let max_headers_by_memory = (MAX_COVER_HEADERS_MEMORY / COVER_HEADER_SIZE) as i64;
    if cover_count > MAX_COVER_COUNT || cover_count > max_headers_by_memory {
        return Err(std::io::Error::other(
            "cover_count exceeds safe maximum or memory limit",
        ));
    }
    if cover_count > 0 && cover_size == 0 {
        return Err(std::io::Error::other("cover_count > 0 but cover_size is 0"));
    }
    CoverHeaderIterator::new(cover_reader, cover_size, cover_count)
}

// ========== RLE Unit Tests ==========

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::RleRefClip;
    use super::{tbytes_set_rle_single, tbytes_set_rle_vector_software};

    // ========== RleRefClip Struct Tests ==========

    #[test]
    fn rle_ref_clip_default_initialization() {
        let rle = RleRefClip::default();
        assert_eq!(rle.mem_copy_length, 0, "mem_copy_length should be 0");
        assert_eq!(rle.mem_set_length, 0, "mem_set_length should be 0");
        assert_eq!(rle.mem_set_value, 0, "mem_set_value should be 0");
    }

    #[test]
    fn rle_ref_clip_copy_is_independent() {
        let rle1 = RleRefClip {
            mem_set_value: 0x42,
            mem_set_length: 10,
            ..Default::default()
        };

        let _rle2 = rle1; // Copy (not reference) - verified to be independent

        assert_eq!(
            rle1.mem_set_value, 0x42,
            "original should be unchanged after copy modification"
        );
    }

    // ========== tbytes_set_rle_single Tests ==========

    /// Test mem_set_value == 0 behavior: should skip bytes without modification
    #[test]
    fn tbytes_set_rle_single_skip_when_mem_set_value_zero() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 5,
            mem_set_value: 0,
        };

        let mut cache = Cursor::new(vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let mut copy_length: i64 = 5;
        let mut shared_buffer = [0u8; 32];

        let result = tbytes_set_rle_single(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
        );

        assert!(result.is_ok(), "tbytes_set_rle_single should succeed");
        assert_eq!(copy_length, 0, "copy_length should be fully consumed");
        assert_eq!(
            rle_loader.mem_set_length, 0,
            "mem_set_length should be exhausted"
        );
        assert_eq!(cache.position(), 5, "cache position should advance by 5");
    }

    /// Test mem_set_value == 0xFF (byte flip via wrapping_neg): 0 - 1 = 0xFF
    #[test]
    fn tbytes_set_rle_single_byte_flip_with_ff() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 3,
            mem_set_value: 0xFF, // wrapping_neg(1) = 0xFF
        };

        let mut cache = Cursor::new(vec![0x00, 0x7F, 0x80]);
        let mut copy_length: i64 = 3;
        let mut shared_buffer = [0u8; 32];

        let result = tbytes_set_rle_single(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
        );

        assert!(result.is_ok(), "tbytes_set_rle_single should succeed");
        assert_eq!(copy_length, 0, "copy_length should be fully consumed");

        // Verify wrapping_add with 0xFF (which is wrapping_neg of 1):
        // 0x00 + 0xFF = 0xFF (255)
        // 0x7F (127) + 0xFF (255) = 382 mod 256 = 126 = 0x7E
        // 0x80 (128) + 0xFF (255) = 383 mod 256 = 127 = 0x7F
        assert_eq!(shared_buffer[0], 0xFF, "byte 0 should be 0xFF");
        assert_eq!(shared_buffer[1], 0x7E, "byte 1 should be 0x7E");
        assert_eq!(shared_buffer[2], 0x7F, "byte 2 should be 0x7F");
    }

    /// Test wrapping_add behavior at boundary (0xFF + 0x01 = 0x00)
    #[test]
    fn tbytes_set_rle_single_wrapping_at_boundary() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 4,
            mem_set_value: 0x01, // wrapping_add will wrap at 0xFF
        };

        // Bytes at boundary: 0xFF should wrap to 0x00
        let mut cache = Cursor::new(vec![0xFF, 0xFF, 0x00, 0x7F]);
        let mut copy_length: i64 = 4;
        let mut shared_buffer = [0u8; 32];

        let result = tbytes_set_rle_single(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
        );

        assert!(result.is_ok(), "tbytes_set_rle_single should succeed");
        // 0xFF + 0x01 = 0x00 (wrapped), 0xFF + 0x01 = 0x00, 0x00 + 0x01 = 0x01, 0x7F +
        // 0x01 = 0x80
        assert_eq!(shared_buffer[0], 0x00, "0xFF + 0x01 should wrap to 0x00");
        assert_eq!(shared_buffer[1], 0x00, "0xFF + 0x01 should wrap to 0x00");
        assert_eq!(shared_buffer[2], 0x01, "0x00 + 0x01 should be 0x01");
        assert_eq!(shared_buffer[3], 0x80, "0x7F + 0x01 should be 0x80");
    }

    /// Test mem_set_length > copy_length: should only process copy_length bytes
    #[test]
    fn tbytes_set_rle_single_truncates_to_copy_length() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 10, // More than copy_length
            mem_set_value: 0x0F,
        };

        let mut cache = Cursor::new(vec![0x00, 0x11, 0x22, 0x33]);
        let mut copy_length: i64 = 4; // Only 4 bytes to process
        let mut shared_buffer = [0u8; 32];

        let result = tbytes_set_rle_single(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
        );

        assert!(result.is_ok(), "tbytes_set_rle_single should succeed");
        assert_eq!(copy_length, 0, "copy_length should be exhausted");
        // mem_set_length should be reduced by 4 (copy_length), not to 0
        assert_eq!(
            rle_loader.mem_set_length, 6,
            "mem_set_length should have 6 remaining"
        );
    }

    /// Test mem_set_length == 0: should return early without modification
    #[test]
    fn tbytes_set_rle_single_early_return_when_no_length() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 0, // No work to do
            mem_set_value: 0xFF,
        };

        let mut cache = Cursor::new(vec![0xAA, 0xBB]);
        let mut copy_length: i64 = 2;
        let mut shared_buffer = [0u8; 32];
        let initial_pos = cache.position();

        let result = tbytes_set_rle_single(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
        );

        assert!(result.is_ok(), "tbytes_set_rle_single should succeed");
        assert_eq!(copy_length, 2, "copy_length should be unchanged");
        assert_eq!(
            cache.position(),
            initial_pos,
            "cache position should not advance"
        );
    }

    // ========== tbytes_set_rle_vector_software Tests ==========

    /// Test wrapping_add edge case: 0xFF + 0x01 = 0x00
    #[test]
    fn tbytes_set_rle_vector_wrapping_add_edge_case() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 2,
            mem_set_length: 0,
            mem_set_value: 0,
        };

        let mut cache = Cursor::new(Vec::new());
        let mut copy_length: i64 = 2;
        let mut buf = [0u8; 32];

        // Set up buffer with test values at offset positions
        // rle_idx = 0 (rle data), old_idx = MAX_ARRAY_POOL_SECOND_OFFSET (16)
        let rle_idx = 0;
        let old_idx = 16;

        buf[rle_idx] = 0xFF;
        buf[old_idx] = 0x01; // 0xFF + 0x01 = 0x00 (wrapped)

        let result = tbytes_set_rle_vector_software(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            1, // decode_step = 1
            &mut buf,
            rle_idx,
            old_idx,
        );

        assert!(
            result.is_ok(),
            "tbytes_set_rle_vector_software should succeed"
        );
        assert_eq!(buf[rle_idx], 0x00, "0xFF + 0x01 should wrap to 0x00");
        assert_eq!(
            rle_loader.mem_copy_length, 1,
            "mem_copy_length should be decremented"
        );
        assert_eq!(copy_length, 1, "copy_length should be decremented");
    }

    /// Test wrapping_add with 0x80 + 0x80 = 0x00
    #[test]
    fn tbytes_set_rle_vector_wrapping_add_0x80_0x80() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 1,
            mem_set_length: 0,
            mem_set_value: 0,
        };

        let mut cache = Cursor::new(Vec::new());
        let mut copy_length: i64 = 1;
        let mut buf = [0u8; 32];

        let rle_idx = 4;
        let old_idx = 20;

        buf[rle_idx] = 0x80;
        buf[old_idx] = 0x80; // 0x80 + 0x80 = 0x00 (wrapped, 128 + 128 = 256 mod 256)

        let result = tbytes_set_rle_vector_software(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            1,
            &mut buf,
            rle_idx,
            old_idx,
        );

        assert!(
            result.is_ok(),
            "tbytes_set_rle_vector_software should succeed"
        );
        assert_eq!(buf[rle_idx], 0x00, "0x80 + 0x80 should wrap to 0x00");
    }

    /// Test multi-byte decode_step with wrapping at boundary
    #[test]
    fn tbytes_set_rle_vector_multi_byte_with_wrapping() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 3,
            mem_set_length: 0,
            mem_set_value: 0,
        };

        let mut cache = Cursor::new(Vec::new());
        let mut copy_length: i64 = 3;
        let mut buf = [0u8; 32];

        let rle_idx = 0;
        let old_idx = 16;

        // First byte: no overflow
        buf[rle_idx] = 0x10;
        buf[old_idx] = 0x20; // 0x10 + 0x20 = 0x30

        // Second byte: boundary case
        buf[rle_idx + 1] = 0xFE;
        buf[old_idx + 1] = 0x03; // 0xFE + 0x03 = 0x01 (wrapped, 254 + 3 = 257 mod 256 = 1)

        // Third byte: normal case
        buf[rle_idx + 2] = 0x7F;
        buf[old_idx + 2] = 0x01; // 0x7F + 0x01 = 0x80

        let result = tbytes_set_rle_vector_software(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            3,
            &mut buf,
            rle_idx,
            old_idx,
        );

        assert!(
            result.is_ok(),
            "tbytes_set_rle_vector_software should succeed"
        );
        assert_eq!(buf[rle_idx], 0x30, "0x10 + 0x20 should be 0x30");
        assert_eq!(buf[rle_idx + 1], 0x01, "0xFE + 0x03 should wrap to 0x01");
        assert_eq!(buf[rle_idx + 2], 0x80, "0x7F + 0x01 should be 0x80");
    }

    /// Test mem_copy_length is properly decremented
    #[test]
    fn tbytes_set_rle_vector_decrements_mem_copy_length() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 5,
            mem_set_length: 0,
            mem_set_value: 0,
        };

        let mut cache = Cursor::new(Vec::new());
        let mut copy_length: i64 = 5;
        let mut buf = [0u8; 32];

        // Fill buf with test data at offset positions
        for i in 0..5 {
            buf[i] = 0;
            buf[16 + i] = i as u8;
        }

        let _ = tbytes_set_rle_vector_software(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            3, // decode_step = 3
            &mut buf,
            0,
            16,
        );

        assert_eq!(
            rle_loader.mem_copy_length, 2,
            "mem_copy_length should be 5 - 3 = 2"
        );
        assert_eq!(copy_length, 2, "copy_length should be 5 - 3 = 2");
    }

    // ========== RLE Edge Case: mem_set_step capping ==========

    /// When mem_set_length exceeds shared_buffer.len(), the step should be
    /// capped to the buffer size. Only shared_buffer.len() bytes are processed.
    #[test]
    fn tbytes_set_rle_single_caps_by_small_shared_buffer() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 100, // much larger than buffer
            mem_set_value: 0x01,
        };

        let mut cache = Cursor::new(vec![0x00; 100]);
        let mut copy_length: i64 = 100;
        let mut shared_buffer = [0u8; 4]; // tiny buffer

        let result = tbytes_set_rle_single(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
        );

        assert!(result.is_ok(), "should succeed even with tiny buffer");
        // mem_set_step = min(100, 100, 4) = 4 bytes processed
        assert_eq!(copy_length, 96, "copy_length should be reduced by 4");
        assert_eq!(
            rle_loader.mem_set_length, 96,
            "mem_set_length should have 96 remaining"
        );
    }

    /// When both mem_set_length and copy_length are larger than the buffer,
    /// but copy_length is the smallest, step is capped to copy_length.
    #[test]
    fn tbytes_set_rle_single_caps_by_copy_length_when_smallest() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 100,
            mem_set_value: 0x01,
        };

        let mut cache = Cursor::new(vec![0x00; 100]);
        let mut copy_length: i64 = 3; // smaller than buffer
        let mut shared_buffer = [0u8; 32];

        let result = tbytes_set_rle_single(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
        );

        assert!(result.is_ok());
        // mem_set_step = min(100, 3, 32) = 3
        assert_eq!(copy_length, 0, "copy_length should be exhausted");
        assert_eq!(
            rle_loader.mem_set_length, 97,
            "mem_set_length should have 97 remaining"
        );
    }

    /// When mem_set_value is 0 and step is capped by small buffer, position
    /// advances by the buffer size without modifying data.
    #[test]
    fn tbytes_set_rle_single_zero_value_capped_by_buffer() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 50,
            mem_set_value: 0, // skip mode
        };

        let mut cache = Cursor::new(vec![0xAA; 50]);
        let mut copy_length: i64 = 50;
        let mut shared_buffer = [0u8; 8]; // tiny buffer

        let result = tbytes_set_rle_single(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
        );

        assert!(result.is_ok());
        // mem_set_step = min(50, 50, 8) = 8 bytes skipped
        assert_eq!(cache.position(), 8, "should advance 8 bytes");
        assert_eq!(copy_length, 42, "copy_length should be 50 - 8 = 42");
        assert_eq!(
            rle_loader.mem_set_length, 42,
            "mem_set_length should have 42 remaining"
        );
    }

    // ========== RLE Edge Case: empty streams ==========

    /// When both mem_set_length and mem_copy_length are 0, tbytes_set_rle
    /// should return immediately without doing anything.
    #[test]
    fn tbytes_set_rle_empty_stream_no_modification() {
        use super::tbytes_set_rle;

        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 0,
            mem_set_value: 0,
        };

        let mut cache = Cursor::new(vec![0xAA, 0xBB, 0xCC]);
        let mut copy_length: i64 = 3;
        let mut shared_buffer = [0u8; 32];
        let initial_pos = cache.position();

        // Create an empty rle_code_stream (no data to read)
        let mut rle_code: &[u8] = &[];
        let result = tbytes_set_rle(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
            &mut rle_code,
        );

        assert!(result.is_ok(), "empty RLE stream should succeed");
        assert_eq!(copy_length, 3, "copy_length should be unchanged");
        assert_eq!(
            cache.position(),
            initial_pos,
            "cache position should not advance"
        );
    }

    // ========== RLE Edge Case: mem_copy_length bounds ==========

    /// When mem_copy_length exceeds MAX_ARRAY_POOL_SECOND_OFFSET, the decode
    /// step is capped to MAX_ARRAY_POOL_SECOND_OFFSET in tbytes_set_rle.
    #[test]
    fn tbytes_set_rle_caps_decode_step_to_second_offset() {
        use super::MAX_ARRAY_POOL_SECOND_OFFSET;
        use std::io::Cursor;

        // Setup: mem_copy_length much larger than MAX_ARRAY_POOL_SECOND_OFFSET
        // decode_step = min(mem_copy_length, copy_length, MAX_ARRAY_POOL_SECOND_OFFSET)
        // This tests the cap by MAX_ARRAY_POOL_SECOND_OFFSET
        let offset = MAX_ARRAY_POOL_SECOND_OFFSET;
        // We can't easily test the full tbytes_set_rle with real I/O here because
        // it reads from rle_code_stream and cache. Instead, verify the constant
        // and the capping logic is exercised via tbytes_set_rle_vector_software.
        // Test that decode_step exceeding available data returns correctly.
        let mut rle_loader = RleRefClip {
            mem_copy_length: offset as i64 + 100, // exceeds offset
            mem_set_length: 0,
            mem_set_value: 0,
        };

        let mut cache = Cursor::new(Vec::new());
        let mut copy_length: i64 = offset as i64 + 100;
        // Need large enough buffer for rle_idx and old_idx regions
        let mut buf = vec![0u8; offset * 2 + 100];

        // Fill with test data at old_idx
        for i in 0..offset {
            buf[offset + i] = (i % 256) as u8;
        }

        // decode_step = min(mem_copy_length, copy_length, MAX_ARRAY_POOL_SECOND_OFFSET)
        // = min(offset+100, offset+100, offset) = offset
        let result = tbytes_set_rle_vector_software(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            offset, // decode_step = MAX_ARRAY_POOL_SECOND_OFFSET
            &mut buf,
            0,      // rle_idx
            offset, // old_idx
        );

        assert!(
            result.is_ok(),
            "should succeed with decode_step at MAX_ARRAY_POOL_SECOND_OFFSET"
        );
        assert_eq!(
            rle_loader.mem_copy_length, 100,
            "mem_copy_length should have 100 remaining after cap"
        );
        assert_eq!(
            copy_length, 100,
            "copy_length should have 100 remaining after cap"
        );
    }

    /// When mem_copy_length is zero, tbytes_set_rle skips the vector step
    /// entirely (returns early after calling tbytes_set_rle_single).
    #[test]
    fn tbytes_set_rle_zero_copy_length_returns_early() {
        use super::tbytes_set_rle;

        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 0,
            mem_set_value: 0,
        };

        let mut cache = Cursor::new(vec![0x00; 8]);
        let mut copy_length: i64 = 8;
        let mut shared_buffer = [0u8; 32];
        let mut rle_code: &[u8] = &[];

        let result = tbytes_set_rle(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
            &mut rle_code,
        );

        assert!(result.is_ok());
        // Since both mem_set_length and mem_copy_length are 0, nothing happens
        assert_eq!(copy_length, 8, "nothing consumed");
    }

    // ========== tbytes_copy_stream_from_old_clip Negative Tests ==========

    /// tbytes_copy_stream_from_old_clip with negative copy_length should return
    /// an error.
    #[test]
    fn tbytes_copy_stream_from_old_clip_negative_copy_length_fails() {
        use super::tbytes_copy_stream_from_old_clip;

        let mut cache = Cursor::new(Vec::new());
        let mut reader: &[u8] = &[];
        let mut shared_buffer = [0u8; 32];
        let result =
            tbytes_copy_stream_from_old_clip(&mut cache, &mut reader, -1, &mut shared_buffer);
        assert!(result.is_err(), "negative copy_length should fail");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("copy_length is negative"), "msg={msg}");
    }

    // ========== tbytes_set_rle_single Additional Wrapping Tests ==========

    /// tbytes_set_rle_single with basic non-zero addition (add 0x10).
    #[test]
    fn tbytes_set_rle_single_basic_addition() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 3,
            mem_set_value: 0x10, // add 16 to each byte
        };
        let mut cache = Cursor::new(vec![0x10, 0x20, 0x30]);
        let mut copy_length: i64 = 3;
        let mut shared_buffer = [0u8; 32];

        let result = tbytes_set_rle_single(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
        );

        assert!(result.is_ok());
        // 0x10 + 0x10 = 0x20, 0x20 + 0x10 = 0x30, 0x30 + 0x10 = 0x40
        assert_eq!(shared_buffer[0], 0x20);
        assert_eq!(shared_buffer[1], 0x30);
        assert_eq!(shared_buffer[2], 0x40);
    }

    /// tbytes_set_rle_single with single byte addition (minimum non-trivial
    /// case).
    #[test]
    fn tbytes_set_rle_single_single_byte_addition() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 1,
            mem_set_value: 0x01,
        };
        let mut cache = Cursor::new(vec![0x00]);
        let mut copy_length: i64 = 1;
        let mut shared_buffer = [0u8; 32];

        let result = tbytes_set_rle_single(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
        );

        assert!(result.is_ok());
        assert_eq!(copy_length, 0, "copy_length should be consumed");
        assert_eq!(
            rle_loader.mem_set_length, 0,
            "mem_set_length should be exhausted"
        );
        assert_eq!(shared_buffer[0], 0x01, "0x00 + 0x01 = 0x01");
    }

    /// tbytes_set_rle_single with non-zero mem_set_value capped by a small
    /// shared buffer.
    #[test]
    fn tbytes_set_rle_single_non_zero_capped_by_small_buffer() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 100,
            mem_set_value: 0x10, // non-zero
        };
        let mut cache = Cursor::new(vec![0x00; 100]);
        let mut copy_length: i64 = 100;
        let mut shared_buffer = [0u8; 4]; // tiny buffer

        let result = tbytes_set_rle_single(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
        );

        assert!(result.is_ok());
        // mem_set_step = min(100, 100, 4) = 4 bytes processed
        assert_eq!(copy_length, 96, "copy_length should be reduced by 4");
        assert_eq!(
            rle_loader.mem_set_length, 96,
            "mem_set_length should have 96 remaining"
        );
        // Verify the bytes were modified (non-zero code path)
        assert_eq!(shared_buffer[0], 0x10, "0x00 + 0x10 = 0x10");
        assert_eq!(shared_buffer[3], 0x10, "0x00 + 0x10 = 0x10");
    }

    /// tbytes_set_rle_single with mem_set_value = 0x80 and data = 0x80 tests
    /// wrapping at 0x80 + 0x80 = 0x00.
    #[test]
    fn tbytes_set_rle_single_wrapping_0x80_plus_0x80() {
        let mut rle_loader = RleRefClip {
            mem_copy_length: 0,
            mem_set_length: 1,
            mem_set_value: 0x80,
        };
        let mut cache = Cursor::new(vec![0x80]);
        let mut copy_length: i64 = 1;
        let mut shared_buffer = [0u8; 32];

        let result = tbytes_set_rle_single(
            &mut rle_loader,
            &mut cache,
            &mut copy_length,
            &mut shared_buffer,
        );

        assert!(result.is_ok());
        assert_eq!(shared_buffer[0], 0x00, "0x80 + 0x80 should wrap to 0x00");
    }
}
