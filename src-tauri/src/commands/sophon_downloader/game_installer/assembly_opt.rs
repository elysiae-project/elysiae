//! Optimized assembly helpers using memory-mapped I/O and zero-copy techniques.
//!
//! These functions are designed to match the performance of the original Sophon
//! DLL's assembly pipeline, which uses memory-mapped files and large buffers.

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

/// Trait alias for write + seek operations.
pub trait WriteSeek: Write + Seek {}
impl<T: Write + Seek> WriteSeek for T {}

/// Memory-mapped file reader for efficient old file chunk reuse.
pub struct MmapReader {
    mmap: memmap2::Mmap,
}

impl MmapReader {
    /// Create a new memory-mapped reader for the given file.
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Ok(Self { mmap })
    }

    /// Read bytes starting from the given offset.
    pub fn read_at(&self, offset: usize, len: usize) -> &[u8] {
        let end = (offset + len).min(self.mmap.len());
        &self.mmap[offset..end]
    }

    /// Get the total length of the mapped file.
    pub fn len(&self) -> usize {
        self.mmap.len()
    }
}

/// Write a chunk from an old file to the output using memory-mapped I/O.
/// This is significantly faster than buffered I/O for large chunks.
pub fn write_chunk_from_mmap(
    old_file_path: &Path,
    writer: &mut dyn WriteSeek,
    new_offset: u64,
    old_offset: u64,
    expected_size: u64,
    file_hasher: Option<&mut Md5>,
    chunk_decompressed_hash_md5: &str,
) -> SophonResult<u64> {
    let reader = MmapReader::new(old_file_path).map_err(SophonError::Io)?;

    writer.seek(SeekFrom::Start(new_offset))?;

    let old_offset = old_offset as usize;
    let expected_size = expected_size as usize;

    if old_offset + expected_size > reader.len() {
        return Err(SophonError::SizeMismatch {
            item: old_file_path.display().to_string(),
            expected: expected_size as u64,
            actual: (reader.len() - old_offset) as u64,
        });
    }

    let chunk_data = reader.read_at(old_offset, expected_size);

    let mut chunk_hasher = Md5::new();
    chunk_hasher.update(chunk_data);

    if let Some(hasher) = file_hasher {
        hasher.update(chunk_data);
    }

    writer.write_all(chunk_data)?;

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

    Ok(expected_size as u64)
}

/// Optimized decompression using a large buffer for better throughput.
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
    fn mmap_reader_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, b"hello world").unwrap();

        let reader = MmapReader::new(&path).unwrap();
        assert_eq!(reader.len(), 11);

        let data = reader.read_at(0, 5);
        assert_eq!(data, b"hello");
    }
}
