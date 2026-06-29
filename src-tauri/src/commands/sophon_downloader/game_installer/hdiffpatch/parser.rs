use std::io::Read;

#[allow(dead_code)]
pub(crate) trait BinaryExtensions: Read {
    fn read_string_to_null(&mut self, buffer_size: usize) -> std::io::Result<String> {
        let mut buf = Vec::with_capacity(buffer_size.min(64));
        let mut byte = [0u8; 1];
        loop {
            let n = self.read(&mut byte)?;
            if n == 0 || byte[0] == 0 {
                break;
            }
            if buf.len() >= buffer_size {
                return Err(std::io::Error::other(
                    "null byte not found within buffer_size",
                ));
            }
            buf.push(byte[0]);
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    fn read_long_7bit(&mut self) -> std::io::Result<i64> {
        self.read_long_7bit_tagged(0, 0)
    }

    fn read_long_7bit_tagged(&mut self, tag_bit: u8, prev_byte: u8) -> std::io::Result<i64> {
        let code = if tag_bit != 0 {
            prev_byte
        } else {
            let mut b = [0u8; 1];
            self.read_exact(&mut b)?;
            b[0]
        };
        let mask = (1u8 << (7 - tag_bit)).wrapping_sub(1);
        let mut value = (code & mask) as i64;
        if (code & (1 << (7 - tag_bit))) == 0 {
            return Ok(value);
        }
        loop {
            if value > (i64::MAX >> 7) {
                return Err(std::io::Error::other("varint overflow"));
            }
            let mut b = [0u8; 1];
            self.read_exact(&mut b)?;
            let code = b[0];
            value = (value << 7) | ((code & 0x7F) as i64);
            if (code & 0x80) == 0 {
                break;
            }
        }
        Ok(value)
    }

    fn read_int_7bit(&mut self) -> std::io::Result<i32> {
        self.read_int_7bit_tagged(0, 0)
    }

    fn read_int_7bit_tagged(&mut self, tag_bit: u8, prev_byte: u8) -> std::io::Result<i32> {
        let code = if tag_bit != 0 {
            prev_byte
        } else {
            let mut b = [0u8; 1];
            self.read_exact(&mut b)?;
            b[0]
        };
        let mask = (1u8 << (7 - tag_bit)).wrapping_sub(1);
        let mut value = (code & mask) as i32;
        if (code & (1 << (7 - tag_bit))) == 0 {
            return Ok(value);
        }
        loop {
            if value > (i32::MAX >> 7) {
                return Err(std::io::Error::other("varint overflow"));
            }
            let mut b = [0u8; 1];
            self.read_exact(&mut b)?;
            let code = b[0];
            value = (value << 7) | ((code & 0x7F) as i32);
            if (code & 0x80) == 0 {
                break;
            }
        }
        Ok(value)
    }
}

impl<T: Read> BinaryExtensions for T {}

pub(crate) fn read_long_7bit_from_slice(
    buf: &[u8],
    offset: &mut usize,
    tag_bit: u8,
    prev_byte: u8,
) -> std::io::Result<i64> {
    if tag_bit == 0 && *offset >= buf.len() {
        return Err(std::io::Error::other("varint: buffer underflow"));
    }
    let code = if tag_bit != 0 {
        prev_byte
    } else {
        let b = buf[*offset];
        *offset += 1;
        b
    };
    let mask = (1u8 << (7 - tag_bit)).wrapping_sub(1);
    let mut value = (code & mask) as i64;
    if (code & (1 << (7 - tag_bit))) == 0 {
        return Ok(value);
    }
    loop {
        if value > (i64::MAX >> 7) {
            return Err(std::io::Error::other("varint overflow in slice"));
        }
        if *offset >= buf.len() {
            return Err(std::io::Error::other(
                "varint: buffer underflow mid-sequence",
            ));
        }
        let code = buf[*offset];
        *offset += 1;
        value = (value << 7) | ((code & 0x7F) as i64);
        if (code & 0x80) == 0 {
            break;
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{BinaryExtensions, read_long_7bit_from_slice};
    use std::io::Cursor;

    fn src(data: &[u8]) -> Cursor<Vec<u8>> {
        Cursor::new(data.to_vec())
    }

    #[test]
    fn read_string_to_null_normal() {
        let mut c = src(b"hello\x00world");
        assert_eq!(c.read_string_to_null(64).unwrap(), "hello");
    }

    #[test]
    fn read_string_to_null_empty_string() {
        let mut c = src(b"\x00hello");
        assert_eq!(c.read_string_to_null(64).unwrap(), "");
    }

    #[test]
    fn read_string_to_null_with_null_terminator() {
        let mut c = src(b"test\x00");
        assert_eq!(c.read_string_to_null(64).unwrap(), "test");
    }

    #[test]
    fn read_string_to_null_hits_buffer_size() {
        let mut c = src(b"abcdefghij");
        let result = c.read_string_to_null(5);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("null byte not found"),
            "should report buffer_size limit hit"
        );
    }

    #[test]
    fn read_string_to_null_truncated_returns_partial() {
        let mut c = src(b"hi");
        let s = c.read_string_to_null(64).unwrap();
        assert_eq!(s, "hi");
    }

    #[test]
    fn read_string_to_null_empty_input() {
        let mut c = src(b"");
        let s = c.read_string_to_null(64).unwrap();
        assert_eq!(s, "");
    }

    #[test]
    fn read_long_7bit_single_byte_zero() {
        let mut c = src(b"\x00");
        assert_eq!(c.read_long_7bit().unwrap(), 0i64);
    }

    #[test]
    fn read_long_7bit_single_byte_less_than_128() {
        let mut c = src(b"\x2a");
        assert_eq!(c.read_long_7bit().unwrap(), 42i64);
    }

    #[test]
    fn read_long_7bit_single_byte_max_no_continuation() {
        let mut c = src(b"\x7f");
        assert_eq!(c.read_long_7bit().unwrap(), 127i64);
    }

    #[test]
    fn read_long_7bit_two_byte_encoding() {
        let mut c = src(b"\x81\x00");
        assert_eq!(c.read_long_7bit().unwrap(), 128i64);
    }

    #[test]
    fn read_long_7bit_two_byte_leading_zero() {
        let mut c = src(b"\x80\x01");
        assert_eq!(c.read_long_7bit().unwrap(), 1i64);
    }

    #[test]
    fn read_long_7bit_multi_byte_16383() {
        let mut c = src(b"\xff\x7f");
        assert_eq!(c.read_long_7bit().unwrap(), 16383i64);
    }

    #[test]
    fn read_long_7bit_three_byte() {
        let mut c = src(b"\x81\x80\x00");
        assert_eq!(c.read_long_7bit().unwrap(), 16384i64);
    }

    #[test]
    fn read_long_7bit_alternating_continuation() {
        let mut c = src(b"\xa1\x82\x03");
        assert_eq!(c.read_long_7bit().unwrap(), 540931i64);
    }

    #[test]
    fn read_long_7bit_value_at_127_128_boundary() {
        let mut c = src(b"\x7f");
        assert_eq!(c.read_long_7bit().unwrap(), 127);
        let mut c2 = src(b"\x81\x00");
        assert_eq!(c2.read_long_7bit().unwrap(), 128);
    }

    #[test]
    fn read_long_7bit_overflow_returns_error() {
        let data = vec![0xFFu8; 12];
        let mut c = Cursor::new(data);
        let result = c.read_long_7bit();
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("varint overflow"),
            "should report varint overflow"
        );
    }

    #[test]
    fn read_long_7bit_tagged_zero_tag_no_continuation() {
        let mut c = src(b"\x2a");
        assert_eq!(c.read_long_7bit_tagged(0, 0).unwrap(), 42i64);
    }

    #[test]
    fn read_long_7bit_tagged_tag_bit_1_uses_prev_byte() {
        let mut c = src(b"");
        let val = c.read_long_7bit_tagged(1, 0x0Au8).unwrap();
        assert_eq!(val, 10i64);
    }

    #[test]
    fn read_long_7bit_tagged_tag_bit_1_with_continuation() {
        let mut c = src(b"\x55");
        let val = c.read_long_7bit_tagged(1, 0x4Au8).unwrap();
        assert_eq!(val, 1365i64);
    }

    #[test]
    fn read_long_7bit_tagged_zero_tag_with_continuation() {
        let mut c = src(b"\x81\x7f");
        let val = c.read_long_7bit_tagged(0, 0).unwrap();
        assert_eq!(val, 255i64);
    }

    #[test]
    fn read_long_7bit_tagged_tag_bit_1_no_continuation_keeps_offset() {
        let buf = b"\x2a".to_vec();
        let mut c = Cursor::new(buf);
        let val = c.read_long_7bit_tagged(1, 0x0Au8).unwrap();
        assert_eq!(val, 10i64);
        let next = c.read_long_7bit().unwrap();
        assert_eq!(next, 42i64);
    }

    #[test]
    fn read_int_7bit_single_byte_zero() {
        let mut c = src(b"\x00");
        assert_eq!(c.read_int_7bit().unwrap(), 0i32);
    }

    #[test]
    fn read_int_7bit_single_byte() {
        let mut c = src(b"\x2a");
        assert_eq!(c.read_int_7bit().unwrap(), 42i32);
    }

    #[test]
    fn read_int_7bit_single_byte_max() {
        let mut c = src(b"\x7f");
        assert_eq!(c.read_int_7bit().unwrap(), 127i32);
    }

    #[test]
    fn read_int_7bit_two_byte() {
        let mut c = src(b"\x81\x00");
        assert_eq!(c.read_int_7bit().unwrap(), 128i32);
    }

    #[test]
    fn read_int_7bit_multi_byte() {
        let mut c = src(b"\xff\x7f");
        assert_eq!(c.read_int_7bit().unwrap(), 16383i32);
    }

    #[test]
    fn read_int_7bit_overflow_returns_error() {
        let data = vec![0xFFu8; 6];
        let mut c = Cursor::new(data);
        let result = c.read_int_7bit();
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("varint overflow"),
            "should report varint overflow"
        );
    }

    #[test]
    fn read_int_7bit_value_at_127_128_boundary() {
        let mut c = src(b"\x7f");
        assert_eq!(c.read_int_7bit().unwrap(), 127);
        let mut c2 = src(b"\x81\x00");
        assert_eq!(c2.read_int_7bit().unwrap(), 128);
    }

    #[test]
    fn read_int_7bit_tagged_zero_tag() {
        let mut c = src(b"\x2a");
        assert_eq!(c.read_int_7bit_tagged(0, 0).unwrap(), 42i32);
    }

    #[test]
    fn read_int_7bit_tagged_tag_bit_1_uses_prev_byte() {
        let mut c = src(b"");
        let val = c.read_int_7bit_tagged(1, 0x0A).unwrap();
        assert_eq!(val, 10i32);
    }

    #[test]
    fn read_int_7bit_tagged_tag_bit_1_with_continuation() {
        let mut c = src(b"\x55");
        let val = c.read_int_7bit_tagged(1, 0x4A).unwrap();
        assert_eq!(val, 1365i32);
    }

    #[test]
    fn read_int_7bit_tagged_with_continuation_overflow() {
        let data = vec![0xFFu8; 6];
        let mut c = Cursor::new(data);
        let result = c.read_int_7bit_tagged(0, 0);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("varint overflow"),
            "should report varint overflow"
        );
    }

    #[test]
    fn read_long_7bit_from_slice_single_byte() {
        let buf = b"\x2a";
        let mut offset = 0;
        let val = read_long_7bit_from_slice(buf, &mut offset, 0, 0).unwrap();
        assert_eq!(val, 42);
        assert_eq!(offset, 1);
    }

    #[test]
    fn read_long_7bit_from_slice_multi_byte() {
        let buf = b"\xff\x7f";
        let mut offset = 0;
        let val = read_long_7bit_from_slice(buf, &mut offset, 0, 0).unwrap();
        assert_eq!(val, 16383);
        assert_eq!(offset, 2);
    }

    #[test]
    fn read_long_7bit_from_slice_zero_value() {
        let buf = b"\x00";
        let mut offset = 0;
        let val = read_long_7bit_from_slice(buf, &mut offset, 0, 0).unwrap();
        assert_eq!(val, 0);
        assert_eq!(offset, 1);
    }

    #[test]
    fn read_long_7bit_from_slice_value_at_boundary() {
        let mut offset = 0usize;
        assert_eq!(
            read_long_7bit_from_slice(b"\x7f", &mut offset, 0, 0).unwrap(),
            127
        );
        assert_eq!(offset, 1);
        let mut offset2 = 0usize;
        assert_eq!(
            read_long_7bit_from_slice(b"\x81\x00", &mut offset2, 0, 0).unwrap(),
            128
        );
        assert_eq!(offset2, 2);
    }

    #[test]
    fn read_long_7bit_from_slice_overflow() {
        let data = vec![0xFFu8; 12];
        let mut offset = 0;
        let result = read_long_7bit_from_slice(&data, &mut offset, 0, 0);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("overflow"),
            "should report overflow"
        );
    }

    #[test]
    fn read_long_7bit_from_slice_buffer_underflow_at_start() {
        let buf = b"";
        let mut offset = 0;
        let result = read_long_7bit_from_slice(buf, &mut offset, 0, 0);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("buffer underflow"),
            "should report buffer underflow"
        );
    }

    #[test]
    fn read_long_7bit_from_slice_buffer_underflow_mid_sequence() {
        let buf = b"\x80";
        let mut offset = 0;
        let result = read_long_7bit_from_slice(buf, &mut offset, 0, 0);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("buffer underflow"),
            "should report buffer underflow mid-sequence"
        );
    }

    #[test]
    fn read_long_7bit_from_slice_tag_bit_1_uses_prev_byte() {
        let buf = b"\x00";
        let mut offset = 0;
        let val = read_long_7bit_from_slice(buf, &mut offset, 1, 0x0A).unwrap();
        assert_eq!(val, 10);
        assert_eq!(offset, 0);
    }

    #[test]
    fn read_long_7bit_from_slice_tag_bit_1_empty_buf() {
        let buf = b"";
        let mut offset = 0;
        let val = read_long_7bit_from_slice(buf, &mut offset, 1, 0x05).unwrap();
        assert_eq!(val, 5);
        assert_eq!(offset, 0);
    }

    #[test]
    fn read_long_7bit_from_slice_tag_bit_1_empty_buf_continuation_fails() {
        let buf = b"";
        let mut offset = 0;
        let result = read_long_7bit_from_slice(buf, &mut offset, 1, 0x45);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("underflow"),
            "error should mention underflow, got: {}",
            err
        );
    }

    #[test]
    fn read_long_7bit_from_slice_tag_bit_1_with_continuation() {
        let buf = b"\x55";
        let mut offset = 0;
        let val = read_long_7bit_from_slice(buf, &mut offset, 1, 0x4A).unwrap();
        assert_eq!(val, 1365);
        assert_eq!(offset, 1);
    }

    #[test]
    fn read_long_7bit_from_slice_alternating_continuation() {
        let buf = b"\xa1\x82\x03";
        let mut offset = 0;
        let val = read_long_7bit_from_slice(buf, &mut offset, 0, 0).unwrap();
        assert_eq!(val, 540931);
        assert_eq!(offset, 3);
    }

    // ========== Varint truncated stream tests ==========

    /// When the first byte has a continuation bit set but the stream has no
    /// more bytes, read_long_7bit should return an error (UnexpectedEof
    /// from read_exact).
    #[test]
    fn read_long_7bit_truncated_stream_returns_error() {
        let mut c = src(b"\x80"); // continuation bit set, but no follow-up byte
        let result = c.read_long_7bit();
        assert!(result.is_err(), "should fail for truncated varint stream");
    }

    /// Same as above but for i32 variant: truncated multi-byte varint.
    #[test]
    fn read_int_7bit_truncated_stream_returns_error() {
        let mut c = src(b"\x80");
        let result = c.read_int_7bit();
        assert!(result.is_err(), "should fail for truncated varint stream");
    }

    /// read_long_7bit_tagged with tag_bit=1, prev_byte signalling continuation
    /// but the stream has no more bytes.
    #[test]
    fn read_long_7bit_tagged_truncated_stream_returns_error() {
        let mut c = src(b"");
        // prev_byte=0x4A: tag_bit=1 -> bits 0-5 = 0x0A (10), bit 6 set -> continuation
        let result = c.read_long_7bit_tagged(1, 0x4A);
        assert!(
            result.is_err(),
            "should fail when continuation byte is missing from stream"
        );
    }

    /// read_int_7bit_tagged with tag_bit=1, prev_byte signalling continuation
    /// but the stream has no more bytes.
    #[test]
    fn read_int_7bit_tagged_truncated_stream_returns_error() {
        let mut c = src(b"");
        let result = c.read_int_7bit_tagged(1, 0x4A);
        assert!(
            result.is_err(),
            "should fail when continuation byte is missing from stream"
        );
    }

    // ========== Varint with tag_bit=2 ==========

    /// tag_bit=2 means 5 value bits and bit 5 is the continuation flag.
    /// prev_byte=0x1F -> value bits = 0x1F & 0x1F = 31, no continuation.
    #[test]
    fn read_long_7bit_tagged_tag_bit_2_no_continuation() {
        let mut c = src(b"\xFF"); // extra byte should not be consumed
        let val = c.read_long_7bit_tagged(2, 0x1F).unwrap();
        assert_eq!(val, 31i64, "tag_bit=2, prev_byte=0x1F should yield 31");
        // Verify no byte was consumed from the stream
        assert_eq!(c.position(), 0, "no bytes should be read from stream");
    }

    /// tag_bit=2 with continuation: prev_byte=0x25 -> bits 0-4 = 5, bit 5 set.
    /// Next byte = 0x03 (no continuation), so value = (5 << 7) | 3 = 643.
    #[test]
    fn read_long_7bit_tagged_tag_bit_2_with_continuation() {
        let mut c = src(b"\x03");
        let val = c.read_long_7bit_tagged(2, 0x25).unwrap();
        assert_eq!(val, 643i64, "tag_bit=2, prev_byte=0x25 + byte 0x03 = 643");
        assert_eq!(c.position(), 1, "one byte consumed from stream");
    }

    /// tag_bit=2, no continuation for i32 variant.
    #[test]
    fn read_int_7bit_tagged_tag_bit_2_no_continuation() {
        let mut c = src(b"\xFF");
        let val = c.read_int_7bit_tagged(2, 0x1F).unwrap();
        assert_eq!(val, 31i32, "tag_bit=2, prev_byte=0x1F should yield 31");
        assert_eq!(c.position(), 0, "no bytes should be read from stream");
    }

    /// tag_bit=2 with continuation for i32 variant.
    #[test]
    fn read_int_7bit_tagged_tag_bit_2_with_continuation() {
        let mut c = src(b"\x03");
        let val = c.read_int_7bit_tagged(2, 0x25).unwrap();
        assert_eq!(val, 643i32, "tag_bit=2, prev_byte=0x25 + byte 0x03 = 643");
        assert_eq!(c.position(), 1, "one byte consumed from stream");
    }

    /// read_long_7bit_from_slice with tag_bit=2, no continuation.
    #[test]
    fn read_long_7bit_from_slice_tag_bit_2_no_continuation() {
        let buf = b"\x00"; // should not be consumed
        let mut offset = 0;
        let val = read_long_7bit_from_slice(buf, &mut offset, 2, 0x1F).unwrap();
        assert_eq!(val, 31i64, "tag_bit=2, prev_byte=0x1F -> value=31");
        assert_eq!(offset, 0, "no bytes should be consumed from buffer");
    }

    /// read_long_7bit_from_slice with tag_bit=2, continuation present.
    #[test]
    fn read_long_7bit_from_slice_tag_bit_2_with_continuation() {
        let buf = b"\x03\xFF"; // 0x03 = no continuation, value 3
        let mut offset = 0;
        let val = read_long_7bit_from_slice(buf, &mut offset, 2, 0x25).unwrap();
        assert_eq!(val, 643i64, "(5<<7)|3 = 643");
        assert_eq!(offset, 1, "one byte consumed from buffer");
    }

    /// read_long_7bit_tagged overflow with tag_bit=1: a valid prev_byte starts
    /// a sequence that keeps growing until it exceeds i64 capacity.
    #[test]
    fn read_long_7bit_tagged_overflow_returns_error() {
        // With tag_bit=1, prev_byte=0x7F: bits 0-5 = 0x3F (63), bit 6 set ->
        // continuation. Feed 12 more 0xFF bytes to trigger overflow.
        let data = vec![0xFFu8; 12];
        let mut c = Cursor::new(data);
        let result = c.read_long_7bit_tagged(1, 0x7F);
        assert!(
            result.is_err(),
            "should detect varint overflow with tag_bit=1"
        );
        assert!(
            result.unwrap_err().to_string().contains("varint overflow"),
            "should report varint overflow"
        );
    }

    /// read_int_7bit_tagged overflow: tag_bit=1 causes large continuation that
    /// overflows i32.
    #[test]
    fn read_int_7bit_tagged_overflow_returns_error() {
        let data = vec![0xFFu8; 6];
        let mut c = Cursor::new(data);
        let result = c.read_int_7bit_tagged(1, 0x7F);
        assert!(
            result.is_err(),
            "should detect varint overflow with tag_bit=1"
        );
        assert!(
            result.unwrap_err().to_string().contains("varint overflow"),
            "should report varint overflow"
        );
    }

    /// read_long_7bit with multi-byte truncated stream: first continuation byte
    /// consumed, second continuation byte set, but no further data.
    #[test]
    fn read_long_7bit_multi_byte_truncated_returns_error() {
        // 0x81 = continuation bit set, value 1
        // 0x82 = continuation bit set, value 2 ,  but no more bytes follow
        let mut c = src(b"\x81\x82");
        let result = c.read_long_7bit();
        assert!(
            result.is_err(),
            "should fail when varint sequence is truncated mid-stream"
        );
    }

    // ========== Varint edge cases: empty stream ==========

    /// read_long_7bit on a completely empty stream should return UnexpectedEof.
    #[test]
    fn read_long_7bit_empty_stream_returns_error() {
        let mut c = src(b"");
        let result = c.read_long_7bit();
        assert!(result.is_err(), "should fail on empty stream");
    }

    /// read_int_7bit on a completely empty stream should return UnexpectedEof.
    #[test]
    fn read_int_7bit_empty_stream_returns_error() {
        let mut c = src(b"");
        let result = c.read_int_7bit();
        assert!(result.is_err(), "should fail on empty stream");
    }

    // ========== Varint with tag_bit=3 ==========

    /// tag_bit=3 means 4 value bits and bit 4 is the continuation flag.
    /// prev_byte=0x0A -> value bits = 0x0A & 0x0F = 10, no continuation.
    #[test]
    fn read_long_7bit_tagged_tag_bit_3_no_continuation() {
        let mut c = src(b"\xFF");
        let val = c.read_long_7bit_tagged(3, 0x0A).unwrap();
        assert_eq!(val, 10i64, "tag_bit=3, prev_byte=0x0A should yield 10");
        assert_eq!(c.position(), 0, "no bytes should be read from stream");
    }

    /// tag_bit=3 with continuation: prev_byte=0x1A -> bits 0-3 = 10, bit 4 set.
    /// Next byte = 0x7F (no continuation), so value = (10 << 7) | 127 = 1407.
    #[test]
    fn read_long_7bit_tagged_tag_bit_3_with_continuation() {
        let mut c = src(b"\x7F");
        let val = c.read_long_7bit_tagged(3, 0x1A).unwrap();
        assert_eq!(val, 1407i64, "tag_bit=3, prev_byte=0x1A + byte 0x7F = 1407");
        assert_eq!(c.position(), 1, "one byte consumed from stream");
    }

    /// tag_bit=3 overflow: prev_byte has continuation, followed by many
    /// continuation bytes until i64 overflows.
    #[test]
    fn read_long_7bit_tagged_tag_bit_3_overflow_returns_error() {
        let data = vec![0xFFu8; 12];
        let mut c = Cursor::new(data);
        let result = c.read_long_7bit_tagged(3, 0x7F);
        assert!(
            result.is_err(),
            "should detect varint overflow with tag_bit=3"
        );
        assert!(
            result.unwrap_err().to_string().contains("varint overflow"),
            "should report varint overflow"
        );
    }

    /// tag_bit=3, no continuation for i32 variant.
    #[test]
    fn read_int_7bit_tagged_tag_bit_3_no_continuation() {
        let mut c = src(b"\xFF");
        let val = c.read_int_7bit_tagged(3, 0x0A).unwrap();
        assert_eq!(val, 10i32, "tag_bit=3, prev_byte=0x0A should yield 10");
        assert_eq!(c.position(), 0, "no bytes should be read from stream");
    }

    /// tag_bit=3 with continuation for i32 variant.
    #[test]
    fn read_int_7bit_tagged_tag_bit_3_with_continuation() {
        let mut c = src(b"\x7F");
        let val = c.read_int_7bit_tagged(3, 0x1A).unwrap();
        assert_eq!(val, 1407i32, "tag_bit=3, prev_byte=0x1A + byte 0x7F = 1407");
        assert_eq!(c.position(), 1, "one byte consumed from stream");
    }

    /// read_long_7bit_from_slice with tag_bit=3, no continuation.
    #[test]
    fn read_long_7bit_from_slice_tag_bit_3_no_continuation() {
        let buf = b"\x00";
        let mut offset = 0;
        let val = read_long_7bit_from_slice(buf, &mut offset, 3, 0x0A).unwrap();
        assert_eq!(val, 10i64, "tag_bit=3, prev_byte=0x0A -> value=10");
        assert_eq!(offset, 0, "no bytes should be consumed from buffer");
    }

    /// read_long_7bit_from_slice with tag_bit=3, continuation present.
    #[test]
    fn read_long_7bit_from_slice_tag_bit_3_with_continuation() {
        let buf = b"\x7F\xFF";
        let mut offset = 0;
        let val = read_long_7bit_from_slice(buf, &mut offset, 3, 0x1A).unwrap();
        assert_eq!(val, 1407i64, "(10<<7)|127 = 1407");
        assert_eq!(offset, 1, "one byte consumed from buffer");
    }

    /// read_long_7bit_from_slice overflow with tag_bit=3.
    #[test]
    fn read_long_7bit_from_slice_tag_bit_3_overflow_returns_error() {
        let data = vec![0xFFu8; 12];
        let mut offset = 0;
        let result = read_long_7bit_from_slice(&data, &mut offset, 3, 0x7F);
        assert!(result.is_err(), "should detect overflow with tag_bit=3");
        assert!(
            result.unwrap_err().to_string().contains("overflow"),
            "should report overflow"
        );
    }

    /// read_long_7bit_from_slice tag_bit=3 with truncated continuation.
    #[test]
    fn read_long_7bit_from_slice_tag_bit_3_truncated_returns_error() {
        let buf = b"";
        let mut offset = 0;
        let result = read_long_7bit_from_slice(buf, &mut offset, 3, 0x1A);
        assert!(result.is_err(), "should fail on truncated stream");
        assert!(
            result.unwrap_err().to_string().contains("underflow"),
            "should report buffer underflow"
        );
    }

    // ========== read_string_to_null edge cases ==========

    /// read_string_to_null with buffer_size=0 and first byte null returns empty
    /// string.
    #[test]
    fn read_string_to_null_buffer_size_zero_with_null() {
        let mut c = src(b"\x00hello");
        let s = c.read_string_to_null(0).unwrap();
        assert_eq!(s, "");
    }

    /// read_string_to_null with buffer_size=0 and non-null data fails
    /// immediately.
    #[test]
    fn read_string_to_null_buffer_size_zero_no_null_fails() {
        let mut c = src(b"a");
        let result = c.read_string_to_null(0);
        assert!(
            result.is_err(),
            "should fail with buffer_size=0 and non-null data"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("null byte not found"),
            "should report buffer_size limit hit"
        );
    }

    /// read_string_to_null with buffer_size=64 and exactly 64 bytes before null
    /// should succeed (null found at position 64, before the 65th push).
    #[test]
    fn read_string_to_null_buffer_size_64_with_exact_data_succeeds() {
        let s = "a".repeat(64);
        let mut data = s.as_bytes().to_vec();
        data.push(0); // null at position 64
        let mut c = Cursor::new(data);
        let result = c.read_string_to_null(64);
        assert!(result.is_ok(), "64 bytes + null should succeed");
        assert_eq!(result.unwrap().len(), 64);
    }

    /// read_string_to_null with buffer_size=64 and 65 bytes without null should
    /// fail because buf.len() reaches 64 before the null is read.
    #[test]
    fn read_string_to_null_buffer_size_64_string_too_long_fails() {
        let s = "b".repeat(65);
        let mut data = s.as_bytes().to_vec();
        data.push(0); // null at position 65, too late
        let mut c = Cursor::new(data);
        let result = c.read_string_to_null(64);
        assert!(
            result.is_err(),
            "65 bytes without null within buffer_size=64 should fail"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("null byte not found"),
            "should report buffer_size limit hit"
        );
    }

    /// read_string_to_null with buffer_size > 64 reads normally up to
    /// buffer_size (not limited to initial capacity of 64).
    #[test]
    fn read_string_to_null_buffer_size_greater_than_64_succeeds() {
        let s = "c".repeat(80);
        let mut data = s.as_bytes().to_vec();
        data.push(0);
        let mut c = Cursor::new(data);
        let result = c.read_string_to_null(100);
        assert!(
            result.is_ok(),
            "80-byte string with buffer_size=100 should succeed"
        );
        assert_eq!(result.unwrap().len(), 80);
    }

    /// read_string_to_null with buffer_size > 64 and string exceeding
    /// buffer_size by one should fail (101 non-null bytes, buffer_size=100).
    #[test]
    fn read_string_to_null_buffer_size_greater_than_64_too_long_fails() {
        let s = "d".repeat(101);
        let mut data = s.as_bytes().to_vec();
        data.push(0); // null at position 101, past the limit
        let mut c = Cursor::new(data);
        let result = c.read_string_to_null(100);
        assert!(
            result.is_err(),
            "101 bytes without null within buffer_size=100 should fail"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("null byte not found"),
            "should report buffer_size limit hit"
        );
    }
}
