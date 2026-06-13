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
}
