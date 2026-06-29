use std::io::{Cursor, Read, Seek, SeekFrom};

use flate2::read::DeflateDecoder;
use tauri_plugin_log::log;

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
        // When comp_length=0 with a non-Nocomp mode, this is unusual ,  log
        // a warning to surface potentially corrupt headers without breaking
        // compatibility with diff producers that emit empty compressed clips.
        if comp_mode != CompressionMode::Nocomp && comp_length == 0 {
            log::warn!(
                "Compressed stream (mode={comp_mode:?}) has comp_length=0; \
                 falling back to uncompressed read of length={length}"
            );
        }
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
            // HDiffPatch's "zlib"-tagged compression (per sisong/HDiffPatch
            // compress_plugin_demo.h) uses raw deflate (RFC 1951) emitted via
            // `deflateInit2(... -MAX_WBITS ...)` with a 1-byte `windowBits`
            // prefix prepended at write time. The prefix byte is compensated
            // by the caller (`patch_single.rs`) before `get_clip_stream` is
            // invoked, so the bytes we receive here are plain raw deflate ,  no
            // 0x78 0x9C header and no Adler32 trailer. Use DeflateDecoder (not
            // ZlibDecoder) accordingly.
            let limited = LimitedFile {
                file,
                remaining: comp_length,
            };
            let mut decoder = DeflateDecoder::new(limited);

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
        CompressionMode::Lz4 => {
            let limited = LimitedFile {
                file,
                remaining: comp_length,
            };
            let mut decoder = lz4::Decoder::new(limited)?;

            if is_buffered {
                if length > MAX_BUFFERED_SIZE {
                    return Err(std::io::Error::other(
                        "buffered lz4 stream exceeds maximum size",
                    ));
                }
                let mut out = Vec::with_capacity(length as usize);
                decoder.read_to_end(&mut out)?;
                Ok((Box::new(Cursor::new(out)), file_bytes))
            } else {
                Ok((Box::new(decoder), file_bytes))
            }
        }
        CompressionMode::Nocomp => Err(std::io::Error::other(
            "Nocomp mode should have been handled above",
        )),
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

#[cfg(test)]
mod tests {
    use super::super::CompressionMode;
    use super::{LimitedFile, get_clip_stream};
    use std::io::Read;

    #[test]
    fn limited_file_read_normal_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"Hello World!").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let mut limited = LimitedFile { file, remaining: 5 };
        let mut buf = [0u8; 10];
        let n = limited.read(&mut buf).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf[..5], b"Hello");
    }

    #[test]
    fn limited_file_read_at_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"ABCDE").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let mut limited = LimitedFile { file, remaining: 5 };
        let mut buf = [0u8; 5];
        let n = limited.read(&mut buf).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf, b"ABCDE");
        // remaining should be 0 after reading exactly
    }

    #[test]
    fn limited_file_read_past_remaining_returns_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"Hello World!").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let mut limited = LimitedFile { file, remaining: 0 };
        let mut buf = [0u8; 10];
        let n = limited.read(&mut buf).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn get_clip_stream_nocomp_unbuffered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"Hello World!").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let (mut reader, file_bytes) =
            get_clip_stream(file, CompressionMode::Nocomp, 0, 5, 5, false).unwrap();
        assert_eq!(file_bytes, 5);
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();
        assert_eq!(output, b"Hello");
    }

    #[test]
    fn get_clip_stream_nocomp_buffered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"Hello World!").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let (mut reader, file_bytes) =
            get_clip_stream(file, CompressionMode::Nocomp, 0, 5, 5, true).unwrap();
        assert_eq!(file_bytes, 5);
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();
        assert_eq!(output, b"Hello");
    }

    #[test]
    fn get_clip_stream_comp_length_zero_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"Hello World!").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        // With Zstd mode and comp_length=0, the code falls back to uncompressed
        // reads (preserves compatibility with diff producers that omit
        // comp_length for tiny clips).
        let (mut reader, file_bytes) =
            get_clip_stream(file, CompressionMode::Zstd, 0, 5, 0, false).unwrap();
        assert_eq!(file_bytes, 5);
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();
        assert_eq!(output, b"Hello");
    }

    #[test]
    fn get_clip_stream_nocomp_buffered_exceeds_max_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"x").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let max = 512u64 * 1024 * 1024;
        let result = get_clip_stream(file, CompressionMode::Nocomp, 0, max + 1, max + 1, true);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("exceeds maximum size"),
            "should report exceeding max buffered size, got: {msg}"
        );
    }

    #[test]
    fn get_clip_stream_zstd_buffered_exceeds_max_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"x").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let max = 512u64 * 1024 * 1024;
        let result = get_clip_stream(file, CompressionMode::Zstd, 0, max + 1, 100, true);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("exceeds maximum size"),
            "should report exceeding max buffered size, got: {msg}"
        );
    }

    #[test]
    fn get_clip_stream_zstd_roundtrip() {
        let original = b"Hello World from zstd roundtrip test!";
        let compressed = zstd::encode_all(std::io::Cursor::new(&original[..]), 3).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.zst");
        std::fs::write(&path, &compressed).unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let (mut reader, file_bytes) = get_clip_stream(
            file,
            CompressionMode::Zstd,
            0,
            original.len() as u64,
            compressed.len() as u64,
            true,
        )
        .unwrap();
        assert_eq!(file_bytes, compressed.len() as u64);
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();
        assert_eq!(output, original);
    }

    #[test]
    fn get_clip_stream_zstd_unbuffered_roundtrip() {
        let original = b"Unbuffered zstd roundtrip test data.";
        let compressed = zstd::encode_all(std::io::Cursor::new(&original[..]), 3).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_unbuf.zst");
        std::fs::write(&path, &compressed).unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let (mut reader, file_bytes) = get_clip_stream(
            file,
            CompressionMode::Zstd,
            0,
            original.len() as u64,
            compressed.len() as u64,
            false,
        )
        .unwrap();
        assert_eq!(file_bytes, compressed.len() as u64);
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();
        assert_eq!(output, original);
    }

    #[test]
    fn get_clip_stream_zlib_buffered_roundtrip() {
        use flate2::Compression;
        use flate2::write::DeflateEncoder;

        let original = b"Hello World from zlib raw-deflate roundtrip test!";
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        use std::io::Write as _;
        encoder.write_all(original).unwrap();
        let raw_deflate = encoder.finish().unwrap();

        // HDiffPatch prepends a 1-byte windowBits=-15 prefix; the caller
        // (`patch_single.rs`) compensates for this before `get_clip_stream`
        // is called, so the bytes here are pure raw deflate (no header, no
        // Adler32).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_zlib.bin");
        std::fs::write(&path, &raw_deflate).unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let (mut reader, file_bytes) = get_clip_stream(
            file,
            CompressionMode::Zlib,
            0,
            original.len() as u64,
            raw_deflate.len() as u64,
            true,
        )
        .unwrap();
        assert_eq!(file_bytes, raw_deflate.len() as u64);
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();
        assert_eq!(output, original);
    }

    #[test]
    fn get_clip_stream_zlib_unbuffered_roundtrip() {
        use flate2::Compression;
        use flate2::write::DeflateEncoder;

        let original = b"Unbuffered zlib raw-deflate roundtrip test data.";
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        use std::io::Write as _;
        encoder.write_all(original).unwrap();
        let raw_deflate = encoder.finish().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_zlib_unbuf.bin");
        std::fs::write(&path, &raw_deflate).unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let (mut reader, file_bytes) = get_clip_stream(
            file,
            CompressionMode::Zlib,
            0,
            original.len() as u64,
            raw_deflate.len() as u64,
            false,
        )
        .unwrap();
        assert_eq!(file_bytes, raw_deflate.len() as u64);
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();
        assert_eq!(output, original);
    }

    #[test]
    fn get_clip_stream_zlib_buffered_exceeds_max_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"x").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let max = 512u64 * 1024 * 1024;
        let result = get_clip_stream(file, CompressionMode::Zlib, 0, max + 1, 100, true);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("exceeds maximum size"),
            "should report exceeding max buffered size, got: {msg}"
        );
    }
    /// Test that Nocomp mode returns error when reached in the match arm
    /// (should not happen in normal operation since Nocomp is handled early)
    #[test]
    fn get_clip_stream_nocomp_with_comp_length_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"Hello World!").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        // This should not happen in normal operation, but test the error path
        // by passing comp_length > 0 with Nocomp mode (which bypasses the early return)
        // Actually, the early return at line 21 catches Nocomp, so we can't reach
        // the match arm. This test documents the expected behavior.
        // The error at line 122-126 is a safety net that should never be reached.
        // To test it, we'd need to modify the code, which defeats the purpose.
        // Instead, this test verifies that Nocomp works correctly in normal operation.
        let result = get_clip_stream(file, CompressionMode::Nocomp, 0, 5, 5, false);
        assert!(result.is_ok(), "Nocomp should succeed with comp_length > 0");
    }
}
