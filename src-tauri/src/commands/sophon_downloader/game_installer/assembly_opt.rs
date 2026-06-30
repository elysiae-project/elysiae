//! Optimized assembly helpers with zero-copy chunk reading.

use std::cell::RefCell;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use md5::{Digest, Md5};

use super::FILE_WRITE_BUFFER_SIZE;
use super::error::{SophonError, SophonResult};

const ASSEMBLY_BUFFER_SIZE: usize = 256 * 1024;

thread_local! {
    static OPT_BUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Write + seek trait alias.
pub trait WriteSeek: Write + Seek {}
impl<T: Write + Seek> WriteSeek for T {}

/// Write a chunk from an old file using direct `pread` reads. Avoids the
/// page-table overhead of mapping the entire old file into the process.
pub fn write_chunk_from_mmap(
    old_file_path: &Path,
    writer: &mut dyn WriteSeek,
    new_offset: u64,
    old_offset: u64,
    expected_size: u64,
    mut file_hasher: Option<&mut Md5>,
    chunk_decompressed_hash_md5: &str,
) -> SophonResult<u64> {
    use std::os::unix::fs::FileExt;
    let file = File::open(old_file_path).map_err(SophonError::Io)?;
    let file_len = file.metadata().map_err(SophonError::Io)?.len();

    writer.seek(SeekFrom::Start(new_offset))?;

    let expected_size_u = expected_size as usize;

    if old_offset + expected_size > file_len {
        return Err(SophonError::SizeMismatch {
            item: old_file_path.display().to_string(),
            expected: expected_size,
            actual: file_len.saturating_sub(old_offset),
        });
    }

    let mut chunk_hasher = Md5::new();
    let mut buf = OPT_BUFFER.with(|cell| {
        let mut buf = cell.take();
        if buf.capacity() < ASSEMBLY_BUFFER_SIZE {
            buf = Vec::with_capacity(ASSEMBLY_BUFFER_SIZE);
        }
        unsafe { buf.set_len(ASSEMBLY_BUFFER_SIZE) };
        buf
    });

    let mut remaining = expected_size_u;
    while remaining > 0 {
        let to_read = remaining.min(buf.len());
        let n = file
            .read_at(
                &mut buf[..to_read],
                old_offset + (expected_size_u - remaining) as u64,
            )
            .map_err(SophonError::Io)?;
        if n == 0 {
            break;
        }
        chunk_hasher.update(&buf[..n]);
        if let Some(hasher) = file_hasher.as_deref_mut() {
            hasher.update(&buf[..n]);
        }
        writer.write_all(&buf[..n])?;
        remaining -= n;
        buf.clear();
    }

    buf.clear();
    OPT_BUFFER.with(|cell| cell.replace(buf));

    let bytes_written = (expected_size_u - remaining) as u64;
    if bytes_written != expected_size {
        return Err(SophonError::SizeMismatch {
            item: old_file_path.display().to_string(),
            expected: expected_size,
            actual: bytes_written,
        });
    }

    const EMPTY_MD5: &str = "00000000000000000000000000000000";
    if chunk_decompressed_hash_md5.len() == 32 && chunk_decompressed_hash_md5 != EMPTY_MD5 {
        let actual = hex::encode(chunk_hasher.finalize());
        if actual != chunk_decompressed_hash_md5 {
            return Err(SophonError::Md5Mismatch {
                item: old_file_path.display().to_string(),
                expected: chunk_decompressed_hash_md5.to_string(),
                actual,
            });
        }
    }

    Ok(bytes_written)
}

/// Decompress a chunk using a large buffer.
pub fn decompress_chunk_optimized(
    chunk_path: &Path,
    writer: &mut dyn WriteSeek,
    offset: u64,
    expected_size: u64,
    file_hasher: Option<&mut Md5>,
    chunk_decompressed_hash_md5: &str,
) -> SophonResult<u64> {
    let f = File::open(chunk_path)?;
    let buf_reader = BufReader::with_capacity(FILE_WRITE_BUFFER_SIZE, f);
    let mut decoder = zstd::Decoder::new(buf_reader)?;

    let window_log: u32 = if cfg!(target_pointer_width = "64") {
        26
    } else {
        25
    };
    decoder.set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(window_log))?;

    writer.seek(SeekFrom::Start(offset))?;

    let mut bytes_written: u64 = 0;
    let mut chunk_hasher = Md5::new();

    let mut buffer = OPT_BUFFER.with(|cell| {
        let mut buf = cell.take();
        if buf.capacity() < ASSEMBLY_BUFFER_SIZE {
            buf = Vec::with_capacity(ASSEMBLY_BUFFER_SIZE);
        }
        unsafe { buf.set_len(ASSEMBLY_BUFFER_SIZE) };
        buf
    });

    if let Some(hasher) = file_hasher {
        loop {
            let n = decoder.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            chunk_hasher.update(&buffer[..n]);
            hasher.update(&buffer[..n]);
            writer.write_all(&buffer[..n])?;
            bytes_written += n as u64;
        }
    } else {
        loop {
            let n = decoder.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            chunk_hasher.update(&buffer[..n]);
            writer.write_all(&buffer[..n])?;
            bytes_written += n as u64;
        }
    }

    buffer.clear();
    OPT_BUFFER.with(|cell| cell.replace(buffer));

    if bytes_written != expected_size {
        return Err(SophonError::SizeMismatch {
            item: chunk_path.display().to_string(),
            expected: expected_size,
            actual: bytes_written,
        });
    }

    const EMPTY_MD5: &str = "00000000000000000000000000000000";
    if chunk_decompressed_hash_md5.len() == 32 && chunk_decompressed_hash_md5 != EMPTY_MD5 {
        let actual = hex::encode(chunk_hasher.finalize());
        if actual != chunk_decompressed_hash_md5 {
            return Err(SophonError::Md5Mismatch {
                item: chunk_path.display().to_string(),
                expected: chunk_decompressed_hash_md5.to_string(),
                actual,
            });
        }
    }

    Ok(bytes_written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn write_chunk_from_mmap_basic() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.bin");
        std::fs::write(&src, b"hello world").unwrap();

        let mut output = Cursor::new(Vec::new());
        let bytes = write_chunk_from_mmap(&src, &mut output, 0, 0, 11, None, "").unwrap();
        assert_eq!(bytes, 11);
        assert_eq!(&output.into_inner()[..], b"hello world");
    }
}
