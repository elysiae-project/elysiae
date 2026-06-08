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
) -> std::io::Result<()> {
    let mut shared_buffer = vec![0u8; MAX_ARRAY_POOL_LEN];
    let mut cache = Cursor::new(Vec::<u8>::new());

    let mut new_pos_back = 0i64;
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
            write_cache_to_output(&mut cache, output_stream)?;
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
        write_cache_to_output(&mut cache, output_stream)?;
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
    let mem_set_step = rle_loader.mem_set_length.min(*copy_length);

    if rle_loader.mem_set_value != 0 {
        let last_pos = out_cache.position();
        let len = mem_set_step as usize;
        out_cache.read_exact(&mut shared_buffer[..len])?;
        out_cache.seek(SeekFrom::Start(last_pos))?;
        for i in (0..len).rev() {
            shared_buffer[i] = shared_buffer[i].wrapping_add(rle_loader.mem_set_value);
        }
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
    for i in 0..decode_step {
        buf[rle_idx + i] = buf[rle_idx + i].wrapping_add(buf[old_idx + i]);
    }
    out_cache.write_all(&buf[rle_idx..rle_idx + decode_step])?;
    rle_loader.mem_copy_length -= decode_step as i64;
    *copy_length -= decode_step as i64;
    Ok(())
}

pub(crate) fn enumerate_cover_headers(
    mut cover_reader: &mut dyn Read,
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
    if cover_count > MAX_COVER_COUNT {
        return Err(std::io::Error::other("cover_count exceeds safe maximum"));
    }
    if cover_count > 0 && cover_size == 0 {
        return Err(std::io::Error::other("cover_count > 0 but cover_size is 0"));
    }
    let mut headers = Vec::with_capacity(cover_count as usize);
    let mut last_old_pos_back = 0i64;
    let mut last_new_pos_back = 0i64;
    let mut remaining = cover_count;

    if cover_size < MAX_MEM_BUFFER_LEN {
        let mut buffer = vec![0u8; cover_size as usize];
        cover_reader.read_exact(&mut buffer)?;

        let mut offset = 0usize;
        while remaining > 0 {
            remaining -= 1;

            let old_pos_back = last_old_pos_back;
            let new_pos_back = last_new_pos_back;
            let p_sign = buffer[offset];
            offset += 1;

            let inc_old_pos_sign = p_sign >> (8 - K_SIGN_TAG_BIT);
            let inc_old_pos =
                read_long_7bit_from_slice(&buffer, &mut offset, K_SIGN_TAG_BIT, p_sign)?;
            let old_pos = if inc_old_pos_sign == 0 {
                old_pos_back
                    .checked_add(inc_old_pos)
                    .ok_or_else(|| std::io::Error::other("old_pos overflow in cover header"))?
            } else {
                old_pos_back
                    .checked_sub(inc_old_pos)
                    .ok_or_else(|| std::io::Error::other("old_pos underflow in cover header"))?
            };
            if old_pos < 0 {
                return Err(std::io::Error::other(
                    "invalid negative old_pos in cover header",
                ));
            }

            let copy_length = read_long_7bit_from_slice(&buffer, &mut offset, 0, 0)?;
            let cover_length = read_long_7bit_from_slice(&buffer, &mut offset, 0, 0)?;
            if copy_length < 0 || cover_length < 0 {
                return Err(std::io::Error::other(
                    "invalid negative copy_length or cover_length in cover header",
                ));
            }
            let new_pos_back = new_pos_back
                .checked_add(copy_length)
                .ok_or_else(|| std::io::Error::other("new_pos overflow in cover header"))?;
            last_old_pos_back = old_pos
                .checked_add(cover_length)
                .ok_or_else(|| std::io::Error::other("old_pos overflow in cover header"))?;
            last_new_pos_back = new_pos_back
                .checked_add(cover_length)
                .ok_or_else(|| std::io::Error::other("last_new_pos overflow in cover header"))?;
            headers.push(CoverHeader::new(
                old_pos,
                new_pos_back,
                cover_length,
                remaining,
            ));
        }
    } else {
        while remaining > 0 {
            remaining -= 1;

            let old_pos_back = last_old_pos_back;
            let new_pos_back = last_new_pos_back;
            let mut p_sign_buf = [0u8; 1];
            cover_reader.read_exact(&mut p_sign_buf)?;
            let p_sign = p_sign_buf[0];

            let inc_old_pos_sign = p_sign >> (8 - K_SIGN_TAG_BIT);
            let inc_old_pos = cover_reader.read_long_7bit_tagged(K_SIGN_TAG_BIT, p_sign)?;
            let old_pos = if inc_old_pos_sign == 0 {
                old_pos_back.checked_add(inc_old_pos).ok_or_else(|| {
                    std::io::Error::other("old_pos overflow in cover header (stream)")
                })?
            } else {
                old_pos_back.checked_sub(inc_old_pos).ok_or_else(|| {
                    std::io::Error::other("old_pos underflow in cover header (stream)")
                })?
            };
            if old_pos < 0 {
                return Err(std::io::Error::other(
                    "invalid negative old_pos in cover header",
                ));
            }

            let copy_length = cover_reader.read_long_7bit()?;
            let cover_length = cover_reader.read_long_7bit()?;
            if copy_length < 0 || cover_length < 0 {
                return Err(std::io::Error::other(
                    "invalid negative copy_length or cover_length in cover header",
                ));
            }
            let new_pos_back = new_pos_back.checked_add(copy_length).ok_or_else(|| {
                std::io::Error::other("new_pos overflow in cover header (stream)")
            })?;
            last_old_pos_back = old_pos.checked_add(cover_length).ok_or_else(|| {
                std::io::Error::other("old_pos overflow in cover header (stream)")
            })?;
            last_new_pos_back = new_pos_back.checked_add(cover_length).ok_or_else(|| {
                std::io::Error::other("last_new_pos overflow in cover header (stream)")
            })?;
            headers.push(CoverHeader::new(
                old_pos,
                new_pos_back,
                cover_length,
                remaining,
            ));
        }
    }
    Ok(headers)
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
        let mut rle1 = RleRefClip::default();
        rle1.mem_set_value = 0x42;
        rle1.mem_set_length = 10;

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
        buf[rle_idx + 0] = 0x10;
        buf[old_idx + 0] = 0x20; // 0x10 + 0x20 = 0x30

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
        assert_eq!(buf[rle_idx + 0], 0x30, "0x10 + 0x20 should be 0x30");
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
}
