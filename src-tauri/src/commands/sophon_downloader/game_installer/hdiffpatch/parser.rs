use std::io::Read;

#[allow(dead_code)]
pub(crate) trait BinaryExtensions: Read {
    fn read_string_to_null(&mut self, _buffer_size: usize) -> std::io::Result<String> {
        let mut buf = Vec::with_capacity(64);
        let mut byte = [0u8; 1];
        loop {
            let n = self.read(&mut byte)?;
            if n == 0 || byte[0] == 0 {
                break;
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
            if (value >> (8 * 8 - 7)) != 0 {
                return Ok(0);
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
            if (value >> (4 * 4 - 7)) != 0 {
                return Ok(0);
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
) -> i64 {
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
        return value;
    }
    loop {
        if (value >> (8 * 8 - 7)) != 0 {
            return 0;
        }
        let code = buf[*offset];
        *offset += 1;
        value = (value << 7) | ((code & 0x7F) as i64);
        if (code & 0x80) == 0 {
            break;
        }
    }
    value
}
