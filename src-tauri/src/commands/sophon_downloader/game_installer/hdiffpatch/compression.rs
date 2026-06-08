use std::io::{Cursor, Read, Seek, SeekFrom};

use flate2::read::ZlibDecoder;

use super::CompressionMode;

pub(crate) fn get_clip_stream(
    mut file: std::fs::File,
    comp_mode: CompressionMode,
    start: u64,
    length: u64,
    comp_length: u64,
    is_buffered: bool,
) -> std::io::Result<(Box<dyn Read>, u64)> {
    let file_bytes = if comp_length > 0 { comp_length } else { length };
    file.seek(SeekFrom::Start(start))?;

    const MAX_BUFFERED_SIZE: u64 = 512 * 1024 * 1024; // 512 MB

    if comp_mode == CompressionMode::Nocomp || comp_length == 0 {
        if is_buffered {
            if length > MAX_BUFFERED_SIZE {
                return Err(std::io::Error::other(
                    "buffered stream exceeds maximum size",
                ));
            }
            let mut buf = vec![0u8; length as usize];
            file.read_exact(&mut buf)?;
            return Ok((Box::new(Cursor::new(buf)), file_bytes));
        }
        let limited = LimitedFile {
            file,
            remaining: length,
        };
        return Ok((Box::new(limited), file_bytes));
    }

    match comp_mode {
        CompressionMode::Zstd => {
            let window_log: u32 = if cfg!(target_pointer_width = "64") {
                31
            } else {
                30
            };
            let limited = LimitedFile {
                file,
                remaining: comp_length,
            };
            let mut decoder = zstd::stream::read::Decoder::new(limited)?;
            decoder.set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(window_log))?;

            if is_buffered {
                if length > MAX_BUFFERED_SIZE {
                    return Err(std::io::Error::other(
                        "buffered zstd stream exceeds maximum size",
                    ));
                }
                let mut out = Vec::with_capacity(length as usize);
                decoder.read_to_end(&mut out)?;
                Ok((Box::new(Cursor::new(out)), file_bytes))
            } else {
                Ok((Box::new(decoder), file_bytes))
            }
        }
        CompressionMode::Zlib => {
            let limited = LimitedFile {
                file,
                remaining: comp_length,
            };
            let mut decoder = ZlibDecoder::new(limited);

            if is_buffered {
                if length > MAX_BUFFERED_SIZE {
                    return Err(std::io::Error::other(
                        "buffered zlib stream exceeds maximum size",
                    ));
                }
                let mut out = Vec::with_capacity(length as usize);
                decoder.read_to_end(&mut out)?;
                Ok((Box::new(Cursor::new(out)), file_bytes))
            } else {
                Ok((Box::new(decoder), file_bytes))
            }
        }
        CompressionMode::Nocomp => unreachable!("handled above"),
    }
}

struct LimitedFile {
    file: std::fs::File,
    remaining: u64,
}

impl Read for LimitedFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let to_read = buf
            .len()
            .min(self.remaining.try_into().unwrap_or(usize::MAX));
        let n = self.file.read(&mut buf[..to_read])?;
        self.remaining -= n as u64;
        Ok(n)
    }
}
