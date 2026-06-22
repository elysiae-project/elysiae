use std::cell::RefCell;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread_local;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use md5::{Digest, Md5};
use tauri_plugin_log::log;

thread_local! {
    static TRANSFER_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

use super::cache::VerificationEntry;
use super::error::{SophonError, SophonResult};
use super::{FILE_WRITE_BUFFER_SIZE, PROGRESS_UPDATE_INTERVAL_MS};
use crate::commands::sophon_downloader::SophonProgress;
use crate::commands::sophon_downloader::proto_parse::{
    SophonManifestAssetChunk, SophonManifestAssetProperty,
};

#[inline]
pub fn chunk_filename(chunk: &SophonManifestAssetChunk) -> String {
    let mut s = String::with_capacity(chunk.chunk_name.len() + 5);
    s.push_str(&chunk.chunk_name);
    s.push_str(".zstd");
    s
}

#[inline]
pub fn decrement_chunk_refcount(
    chunk_name: &str,
    chunk_refcounts: &DashMap<String, usize>,
    chunks_dir: &Path,
) {
    if !validate_chunk_name(chunk_name) {
        return;
    }
    if let Some(mut count) = chunk_refcounts.get_mut(chunk_name) {
        *count -= 1;
        if *count == 0 {
            drop(count);
            chunk_refcounts.remove(chunk_name);
            let _ = fs::remove_file(chunks_dir.join(format!("{chunk_name}.zstd")));
        }
    }
}

pub fn cleanup_tmp_files(dir: &Path) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            cleanup_tmp_files(&path)?;
        } else if path.extension().map(|ext| ext == "tmp").unwrap_or(false) {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

pub fn validate_chunk_name(chunk_name: &str) -> bool {
    if chunk_name.is_empty() {
        log::warn!("chunk_name is empty, skipping file operation");
        return false;
    }
    if chunk_name.contains('\0') {
        log::warn!("chunk_name contains null byte, skipping file operation");
        return false;
    }
    let mut chars = chunk_name.chars();
    if let (Some(first), Some(':')) = (chars.next(), chars.next())
        && first.is_ascii_alphabetic()
    {
        log::warn!("chunk_name has drive letter, skipping file operation");
        return false;
    }
    if chunk_name.starts_with('/')
        || chunk_name.starts_with('\\')
        || chunk_name.split(&['/', '\\']).any(|c| c == "..")
    {
        log::warn!("chunk_name has path traversal pattern, skipping file operation");
        return false;
    }
    true
}

pub fn validate_asset_name(name: &str) -> SophonResult<()> {
    if name.is_empty() {
        return Err(SophonError::InvalidAssetName(
            "asset_name cannot be empty".into(),
        ));
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return Err(SophonError::PathTraversal(name.into()));
    }
    if name.starts_with("./") || name.starts_with(".\\") {
        return Err(SophonError::PathTraversal(name.into()));
    }
    if name.split(&['/', '\\']).any(|component| component == "..") {
        return Err(SophonError::PathTraversal(name.into()));
    }
    if name.contains('\0') {
        return Err(SophonError::InvalidAssetName(
            "asset_name cannot contain null bytes".into(),
        ));
    }
    let mut chars = name.chars();
    if let (Some(first), Some(':')) = (chars.next(), chars.next())
        && first.is_ascii_alphabetic()
    {
        return Err(SophonError::PathTraversal(name.into()));
    }
    Ok(())
}

struct DecrementGuard<'a> {
    chunks: Vec<String>,
    chunk_refcounts: &'a DashMap<String, usize>,
    chunks_dir: &'a Path,
}

impl Drop for DecrementGuard<'_> {
    fn drop(&mut self) {
        for chunk_name in self.chunks.drain(..) {
            decrement_chunk_refcount(&chunk_name, self.chunk_refcounts, self.chunks_dir);
        }
    }
}

pub fn assemble_file(
    file: &SophonManifestAssetProperty,
    game_dir: &Path,
    chunks_dir: &Path,
    temp_dir: &Path,
    chunk_refcounts: &DashMap<String, usize>,
    verify_cache: &DashMap<String, VerificationEntry>,
) -> SophonResult<()> {
    validate_asset_name(&file.asset_name)?;
    if file.is_directory() {
        return Ok(());
    }
    let target_path = game_dir.join(&file.asset_name);
    // Use hex-encoded hash of the asset name as tmp filename to avoid
    // collisions from path sanitization (e.g. "a/b" and "a_b" both become "a_b"
    // when '/' is replaced with '_', but have different hashes).
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    file.asset_name.hash(&mut hasher);
    let tmp_path = temp_dir.join(format!("{:016x}.tmp", hasher.finish()));

    if target_path.exists() {
        let already_valid = super::cache::check_file_md5_cached(
            &target_path,
            file.asset_size,
            &file.asset_hash_md5,
            game_dir,
            verify_cache,
        )?;

        if already_valid {
            log::debug!(
                "assemble_file: skipping already-valid file '{}' ({} bytes, md5={})",
                file.asset_name,
                file.asset_size,
                file.asset_hash_md5
            );
            for chunk in &file.asset_chunks {
                decrement_chunk_refcount(&chunk.chunk_name, chunk_refcounts, chunks_dir);
            }
            return Ok(());
        }
        log::warn!(
            "assemble_file: file '{}' exists but MD5 mismatch, re-assembling",
            file.asset_name
        );
    }

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let out_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)?;

    out_file.set_len(file.asset_size)?;

    let mut buf_writer = BufWriter::with_capacity(FILE_WRITE_BUFFER_SIZE, out_file);
    let mut total_written: u64 = 0;
    let mut file_hasher = if file.asset_hash_md5.is_empty() {
        if !file.is_directory() {
            log::warn!(
                "File '{}' has no asset_hash_md5; assembled without file-level verification",
                file.asset_name
            );
        }
        None
    } else {
        Some(Md5::new())
    };

    let mut transfer_buffer = TRANSFER_BUF.with(|cell| {
        let mut buf = cell.take();
        if buf.capacity() < FILE_WRITE_BUFFER_SIZE {
            buf = Vec::with_capacity(FILE_WRITE_BUFFER_SIZE);
        }
        // Safety: the entire buffer is overwritten by read() before any byte
        // is observed via write_all(). The buffer is initialized only when
        // first allocated (set to 0 in with_capacity branch above), and
        // kept across calls. Resetting len here lets us skip the zero-fill
        // on the hot path.
        unsafe { buf.set_len(FILE_WRITE_BUFFER_SIZE) };
        buf
    });
    let mut guard = DecrementGuard {
        chunks: Vec::new(),
        chunk_refcounts,
        chunks_dir,
    };

    let mut cursor = 0u64;
    let mut sorted_ranges: Vec<(u64, u64)> = file
        .asset_chunks
        .iter()
        .map(|c| (c.chunk_on_file_offset, c.chunk_size_decompressed))
        .collect();
    sorted_ranges.sort_unstable_by_key(|r| r.0);
    for (off, size) in &sorted_ranges {
        if *off != cursor {
            return Err(SophonError::SizeMismatch {
                item: file.asset_name.clone(),
                expected: file.asset_size,
                actual: *off,
            });
        }
        cursor = off
            .checked_add(*size)
            .ok_or_else(|| SophonError::SizeMismatch {
                item: file.asset_name.clone(),
                expected: file.asset_size,
                actual: cursor,
            })?;
    }
    if cursor != file.asset_size {
        return Err(SophonError::SizeMismatch {
            item: file.asset_name.clone(),
            expected: file.asset_size,
            actual: cursor,
        });
    }

    for chunk in &file.asset_chunks {
        if chunk.chunk_old_offset >= 0 {
            // Chunk-level reuse: read decompressed data from the existing game
            // file at the old offset instead of downloading & decompressing.
            debug_assert!(
                chunk.chunk_old_offset >= 0,
                "chunk_old_offset must be non-negative"
            );
            let bytes_written = write_from_old_file(
                &target_path,
                &mut buf_writer,
                chunk.chunk_on_file_offset,
                chunk.chunk_old_offset as u64,
                chunk.chunk_size_decompressed,
                file_hasher.as_mut(),
                &mut transfer_buffer,
                &chunk.chunk_decompressed_hash_md5,
            )
            .inspect_err(|_| {
                let _ = fs::remove_file(&tmp_path);
            })?;
            total_written += bytes_written;
            // No refcount to decrement — old-source chunks were never
            // downloaded.
        } else {
            if !validate_chunk_name(&chunk.chunk_name) {
                return Err(SophonError::PathTraversal(chunk.chunk_name.clone().into()));
            }
            let chunk_path = chunks_dir.join(chunk_filename(chunk));

            let bytes_written = write_decompressed_chunk_at(
                &chunk_path,
                &mut buf_writer,
                chunk.chunk_on_file_offset,
                chunk.chunk_size_decompressed,
                file_hasher.as_mut(),
                &mut transfer_buffer,
                &chunk.chunk_decompressed_hash_md5,
            )
            .inspect_err(|_| {
                let _ = fs::remove_file(&tmp_path);
            })?;

            total_written += bytes_written;
            guard.chunks.push(chunk.chunk_name.clone());
        }
    }

    buf_writer.flush().map_err(|err| {
        let _ = fs::remove_file(&tmp_path);
        SophonError::Io(err)
    })?;
    let out_file = buf_writer.into_inner().map_err(|err| {
        let _ = fs::remove_file(&tmp_path);
        SophonError::Io(err.into_error())
    })?;
    drop(out_file);

    if total_written != file.asset_size {
        let _ = fs::remove_file(&tmp_path);
        return Err(SophonError::SizeMismatch {
            item: file.asset_name.clone(),
            expected: file.asset_size,
            actual: total_written,
        });
    }

    if let Some(hasher) = file_hasher {
        let actual = hex::encode(hasher.finalize());
        if actual != file.asset_hash_md5 {
            let _ = fs::remove_file(&tmp_path);
            return Err(SophonError::Md5Mismatch {
                item: file.asset_name.clone(),
                expected: file.asset_hash_md5.clone(),
                actual,
            });
        }
    }

    if let Err(err) = fs::rename(&tmp_path, &target_path) {
        if err.raw_os_error() == Some(libc::EXDEV)
            || err.kind() == std::io::ErrorKind::CrossesDevices
        {
            log::warn!("rename EXDEV; falling back to copy + unlink: {err}");
            fs::copy(&tmp_path, &target_path)?;
            let _ = fs::remove_file(&tmp_path);
        } else {
            let _ = fs::remove_file(&tmp_path);
            return Err(SophonError::Io(err));
        }
    }

    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(mut perms) = fs::metadata(&target_path).map(|m| m.permissions()) {
            perms.set_mode(0o644);
            let _ = fs::set_permissions(&target_path, perms);
        }
    }

    transfer_buffer.clear();
    TRANSFER_BUF.with(|cell| cell.replace(transfer_buffer));

    Ok(())
}

fn write_decompressed_chunk_at<W: Write + Seek>(
    chunk_path: &Path,
    writer: &mut W,
    offset: u64,
    expected_size: u64,
    file_hasher: Option<&mut Md5>,
    buffer: &mut [u8],
    chunk_decompressed_hash_md5: &str,
) -> SophonResult<u64> {
    let f = File::open(chunk_path)?;
    let buf_reader = BufReader::with_capacity(FILE_WRITE_BUFFER_SIZE, f);
    let mut decoder = zstd::Decoder::new(buf_reader)?;
    let window_log: u32 = if cfg!(target_pointer_width = "64") {
        31
    } else {
        30
    };
    decoder.set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(window_log))?;

    writer.seek(SeekFrom::Start(offset))?;

    let mut bytes_written: u64 = 0;
    let mut chunk_hasher = Md5::new();

    match file_hasher {
        Some(hasher) => {
            let mut hw = HashWriter {
                inner: writer,
                hasher,
            };
            loop {
                let n = decoder.read(buffer)?;
                if n == 0 {
                    break;
                }
                chunk_hasher.update(&buffer[..n]);
                hw.write_all(&buffer[..n])?;
                bytes_written += n as u64;
            }
        }
        None => loop {
            let n = decoder.read(buffer)?;
            if n == 0 {
                break;
            }
            chunk_hasher.update(&buffer[..n]);
            writer.write_all(&buffer[..n])?;
            bytes_written += n as u64;
        },
    }

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

/// Read decompressed bytes directly from an existing game file (old file) at
/// the given old offset, verify the chunk's decompressed MD5, and write to the
/// output writer at the new file offset. Used for chunk-level reuse during
/// updates.
#[allow(clippy::too_many_arguments)]
fn write_from_old_file<W: Write + Seek>(
    old_file_path: &Path,
    writer: &mut W,
    new_offset: u64,
    old_offset: u64,
    expected_size: u64,
    file_hasher: Option<&mut Md5>,
    buffer: &mut [u8],
    chunk_decompressed_hash_md5: &str,
) -> SophonResult<u64> {
    let f = File::open(old_file_path).map_err(SophonError::Io)?;
    let mut reader = BufReader::with_capacity(FILE_WRITE_BUFFER_SIZE, f);
    reader.seek(SeekFrom::Start(old_offset))?;

    writer.seek(SeekFrom::Start(new_offset))?;

    let mut bytes_written: u64 = 0;
    let mut chunk_hasher = Md5::new();
    let mut remaining = expected_size;

    match file_hasher {
        Some(hasher) => {
            let mut hw = HashWriter {
                inner: writer,
                hasher,
            };
            while remaining > 0 {
                let to_read = remaining.min(buffer.len() as u64) as usize;
                reader.read_exact(&mut buffer[..to_read])?;
                chunk_hasher.update(&buffer[..to_read]);
                hw.write_all(&buffer[..to_read])?;
                bytes_written += to_read as u64;
                remaining = remaining.saturating_sub(to_read as u64);
            }
        }
        None => {
            while remaining > 0 {
                let to_read = remaining.min(buffer.len() as u64) as usize;
                reader.read_exact(&mut buffer[..to_read])?;
                chunk_hasher.update(&buffer[..to_read]);
                writer.write_all(&buffer[..to_read])?;
                bytes_written += to_read as u64;
                remaining = remaining.saturating_sub(to_read as u64);
            }
        }
    }

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

struct HashWriter<'a, W: Write> {
    inner: &'a mut W,
    hasher: &'a mut Md5,
}

impl<W: Write> Write for HashWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buf);
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub struct AssemblyTaskParams {
    pub file_idx: usize,
    pub tmp_dir_idx: usize,
    pub all_files: Arc<Vec<SophonManifestAssetProperty>>,
    pub all_tmp_dirs: Arc<Vec<std::path::PathBuf>>,
    pub game_dir: std::path::PathBuf,
    pub chunks_dir: Arc<std::path::PathBuf>,
    pub chunk_refcounts: Arc<DashMap<String, usize>>,
    pub verify_cache: Arc<DashMap<String, VerificationEntry>>,
    pub assembled_files: Arc<AtomicU64>,
    pub last_assembly_update: Arc<Mutex<Instant>>,
    pub total_files: u64,
}

pub fn run_assembly_task(
    params: AssemblyTaskParams,
    updater: impl Fn(SophonProgress) + Send + Sync + 'static,
) -> SophonResult<()> {
    let AssemblyTaskParams {
        file_idx,
        tmp_dir_idx,
        all_files,
        all_tmp_dirs,
        game_dir,
        chunks_dir,
        chunk_refcounts,
        verify_cache,
        assembled_files,
        last_assembly_update,
        total_files,
    } = params;

    if file_idx >= all_files.len() {
        return Err(SophonError::IndexOutOfBounds {
            kind: "file",
            index: file_idx,
        });
    }
    if tmp_dir_idx >= all_tmp_dirs.len() {
        return Err(SophonError::IndexOutOfBounds {
            kind: "temp dir",
            index: tmp_dir_idx,
        });
    }

    let file = &all_files[file_idx];
    let tmp_dir = &all_tmp_dirs[tmp_dir_idx];

    assemble_file(
        file,
        &game_dir,
        &chunks_dir,
        tmp_dir,
        &chunk_refcounts,
        &verify_cache,
    )
    .map_err(|err| SophonError::AssemblyFailed {
        file: file.asset_name.clone(),
        error: err.to_string(),
    })?;

    let count = assembled_files.fetch_add(1, Ordering::Relaxed) + 1;

    {
        if let Ok(mut lu) = last_assembly_update.try_lock()
            && lu.elapsed() >= Duration::from_millis(PROGRESS_UPDATE_INTERVAL_MS)
        {
            updater(SophonProgress::Assembling {
                assembled_files: count,
                total_files,
            });
            *lu = Instant::now();
        }
    }

    Ok(())
}

pub async fn spawn_assembly_task(
    params: AssemblyTaskParams,
    updater: impl Fn(SophonProgress) + Send + Sync + 'static,
) -> SophonResult<()> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let result = run_assembly_task(params, updater);
        let _ = tx.send(result);
    });
    match rx.await {
        Ok(result) => result,
        Err(_) => Err(SophonError::Io(std::io::Error::other(
            "assembly thread cancelled",
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_asset_name_empty() {
        let result = validate_asset_name("");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SophonError::InvalidAssetName(_)
        ));
    }

    #[test]
    fn validate_asset_name_slash_prefix() {
        let result = validate_asset_name("/etc/passwd");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SophonError::PathTraversal(_)));
    }

    #[test]
    fn validate_asset_name_backslash_prefix() {
        let result = validate_asset_name("\\Windows\\system32");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SophonError::PathTraversal(_)));
    }

    #[test]
    fn validate_asset_name_dotdot() {
        let result = validate_asset_name("foo/../../../etc/passwd");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SophonError::PathTraversal(_)));
    }

    #[test]
    fn validate_asset_name_null_byte() {
        let result = validate_asset_name("foo\0bar");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SophonError::InvalidAssetName(_)
        ));
    }

    #[test]
    fn validate_asset_name_drive_letter() {
        let result = validate_asset_name("C:evil");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SophonError::PathTraversal(_)));
    }

    #[test]
    fn validate_asset_name_valid_relative() {
        assert!(validate_asset_name("GameData/Data.pak").is_ok());
    }

    #[test]
    fn validate_asset_name_valid_nested() {
        assert!(validate_asset_name("a/b/c/file.dat").is_ok());
    }

    #[test]
    fn validate_asset_name_dot_slash_prefix() {
        assert!(validate_asset_name("./etc/passwd").is_err());
        assert!(validate_asset_name(".\\Windows\\system32").is_err());
    }

    #[test]
    fn validate_asset_name_consecutive_dots_allowed() {
        assert!(validate_asset_name("foo..bar").is_ok());
        assert!(validate_asset_name("2.0..hotfix.pak").is_ok());
    }

    #[test]
    fn validate_asset_name_dotdot_component_rejected() {
        assert!(validate_asset_name("../etc/passwd").is_err());
        assert!(validate_asset_name("foo/../bar").is_err());
        assert!(validate_asset_name("a/..").is_err());
    }

    #[test]
    fn chunk_filename_format() {
        let chunk = SophonManifestAssetChunk {
            chunk_name: "abc123".into(),
            chunk_decompressed_hash_md5: String::new(),
            chunk_on_file_offset: 0,
            chunk_size: 0,
            chunk_size_decompressed: 0,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: String::new(),
            chunk_old_offset: -1,
        };
        assert_eq!(chunk_filename(&chunk), "abc123.zstd");
    }

    fn make_chunk_file(chunks_dir: &Path, chunk_name: &str, data: &[u8]) {
        let compressed = zstd::encode_all(data, 0).unwrap();
        fs::write(chunks_dir.join(format!("{chunk_name}.zstd")), &compressed).unwrap();
    }

    fn compute_md5_hex(data: &[u8]) -> String {
        let mut hasher = Md5::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    fn make_chunk(name: &str, offset: u64, decompressed_size: u64) -> SophonManifestAssetChunk {
        SophonManifestAssetChunk {
            chunk_name: name.to_string(),
            chunk_decompressed_hash_md5: String::new(),
            chunk_on_file_offset: offset,
            chunk_size: 0,
            chunk_size_decompressed: decompressed_size,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: String::new(),
            chunk_old_offset: -1,
        }
    }

    #[test]
    fn assemble_file_from_single_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("game");
        let chunks_dir = dir.path().join("chunks");
        let temp_dir = dir.path().join("tmp");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(&chunks_dir).unwrap();
        fs::create_dir_all(&temp_dir).unwrap();

        let original_data = b"hello assembly world!";
        make_chunk_file(&chunks_dir, "chunk0", original_data);
        let md5 = compute_md5_hex(original_data);

        let file = SophonManifestAssetProperty {
            asset_name: "output.bin".to_string(),
            asset_chunks: vec![make_chunk("chunk0", 0, original_data.len() as u64)],
            asset_type: 0,
            asset_size: original_data.len() as u64,
            asset_hash_md5: md5,
        };

        let chunk_refcounts = DashMap::new();
        chunk_refcounts.insert("chunk0".to_string(), 1);
        let verify_cache = DashMap::new();

        assemble_file(
            &file,
            &game_dir,
            &chunks_dir,
            &temp_dir,
            &chunk_refcounts,
            &verify_cache,
        )
        .unwrap();

        let result = fs::read(game_dir.join("output.bin")).unwrap();
        assert_eq!(result, original_data);
    }

    #[test]
    fn assemble_file_from_multiple_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("game");
        let chunks_dir = dir.path().join("chunks");
        let temp_dir = dir.path().join("tmp");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(&chunks_dir).unwrap();
        fs::create_dir_all(&temp_dir).unwrap();

        let data_a = b"AAAA";
        let data_b = b"BBBB";
        let total_size = (data_a.len() + data_b.len()) as u64;
        let mut full_data = Vec::new();
        full_data.extend_from_slice(data_a);
        full_data.extend_from_slice(data_b);

        make_chunk_file(&chunks_dir, "chunkA", data_a);
        make_chunk_file(&chunks_dir, "chunkB", data_b);

        let md5 = compute_md5_hex(&full_data);

        let file = SophonManifestAssetProperty {
            asset_name: "multi.bin".to_string(),
            asset_chunks: vec![
                make_chunk("chunkA", 0, data_a.len() as u64),
                make_chunk("chunkB", data_a.len() as u64, data_b.len() as u64),
            ],
            asset_type: 0,
            asset_size: total_size,
            asset_hash_md5: md5,
        };

        let chunk_refcounts = DashMap::new();
        chunk_refcounts.insert("chunkA".to_string(), 1);
        chunk_refcounts.insert("chunkB".to_string(), 1);
        let verify_cache = DashMap::new();

        assemble_file(
            &file,
            &game_dir,
            &chunks_dir,
            &temp_dir,
            &chunk_refcounts,
            &verify_cache,
        )
        .unwrap();

        let result = fs::read(game_dir.join("multi.bin")).unwrap();
        assert_eq!(&result[..4], data_a);
        assert_eq!(&result[4..8], data_b);
        assert_eq!(result.len(), total_size as usize);
    }

    #[test]
    fn assemble_file_skips_valid_existing() {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("game");
        let chunks_dir = dir.path().join("chunks");
        let temp_dir = dir.path().join("tmp");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(&chunks_dir).unwrap();
        fs::create_dir_all(&temp_dir).unwrap();

        let original_data = b"already here";
        let md5 = compute_md5_hex(original_data);
        let target = game_dir.join("existing.bin");
        fs::write(&target, original_data).unwrap();

        make_chunk_file(&chunks_dir, "chunk_skip", original_data);

        let file = SophonManifestAssetProperty {
            asset_name: "existing.bin".to_string(),
            asset_chunks: vec![make_chunk("chunk_skip", 0, original_data.len() as u64)],
            asset_type: 0,
            asset_size: original_data.len() as u64,
            asset_hash_md5: md5,
        };

        let chunk_refcounts = DashMap::new();
        chunk_refcounts.insert("chunk_skip".to_string(), 1);
        let verify_cache = DashMap::new();

        assemble_file(
            &file,
            &game_dir,
            &chunks_dir,
            &temp_dir,
            &chunk_refcounts,
            &verify_cache,
        )
        .unwrap();

        assert!(!chunk_refcounts.contains_key("chunk_skip"));
        assert!(!chunks_dir.join("chunk_skip.zstd").exists());

        let result = fs::read(&target).unwrap();
        assert_eq!(result, original_data);
    }

    #[test]
    fn assemble_file_reassembles_md5_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("game");
        let chunks_dir = dir.path().join("chunks");
        let temp_dir = dir.path().join("tmp");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(&chunks_dir).unwrap();
        fs::create_dir_all(&temp_dir).unwrap();

        let wrong_data = b"wrong content!";
        let correct_data = b"correct data!!";
        let md5 = compute_md5_hex(correct_data);

        let target = game_dir.join("mismatch.bin");
        fs::write(&target, wrong_data).unwrap();

        make_chunk_file(&chunks_dir, "chunk_fix", correct_data);

        let file = SophonManifestAssetProperty {
            asset_name: "mismatch.bin".to_string(),
            asset_chunks: vec![make_chunk("chunk_fix", 0, correct_data.len() as u64)],
            asset_type: 0,
            asset_size: correct_data.len() as u64,
            asset_hash_md5: md5,
        };

        let chunk_refcounts = DashMap::new();
        chunk_refcounts.insert("chunk_fix".to_string(), 1);
        let verify_cache = DashMap::new();

        assemble_file(
            &file,
            &game_dir,
            &chunks_dir,
            &temp_dir,
            &chunk_refcounts,
            &verify_cache,
        )
        .unwrap();

        let result = fs::read(&target).unwrap();
        assert_eq!(result, correct_data);
    }

    #[test]
    fn decrement_chunk_refcount_to_zero_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let chunks_dir = dir.path().join("chunks");
        fs::create_dir_all(&chunks_dir).unwrap();

        let chunk_file = chunks_dir.join("vanish.zstd");
        fs::write(&chunk_file, b"dummy").unwrap();

        let chunk_refcounts = DashMap::new();
        chunk_refcounts.insert("vanish".to_string(), 1);

        decrement_chunk_refcount("vanish", &chunk_refcounts, &chunks_dir);

        assert!(!chunk_refcounts.contains_key("vanish"));
        assert!(!chunk_file.exists());
    }

    #[test]
    fn decrement_chunk_refcount_nonzero_keeps() {
        let dir = tempfile::tempdir().unwrap();
        let chunks_dir = dir.path().join("chunks");
        fs::create_dir_all(&chunks_dir).unwrap();

        let chunk_file = chunks_dir.join("keep.zstd");
        fs::write(&chunk_file, b"dummy").unwrap();

        let chunk_refcounts = DashMap::new();
        chunk_refcounts.insert("keep".to_string(), 2);

        decrement_chunk_refcount("keep", &chunk_refcounts, &chunks_dir);

        let count = *chunk_refcounts.get("keep").unwrap();
        assert_eq!(count, 1);
        assert!(chunk_file.exists());
    }

    #[test]
    fn cleanup_tmp_files_removes_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.tmp");
        let b = dir.path().join("b.tmp");
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        let c = sub.join("c.tmp");
        fs::write(&a, b"x").unwrap();
        fs::write(&b, b"x").unwrap();
        fs::write(&c, b"x").unwrap();

        cleanup_tmp_files(dir.path()).unwrap();

        assert!(!a.exists());
        assert!(!b.exists());
        assert!(!c.exists());
    }

    #[test]
    fn cleanup_tmp_files_skips_non_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let keep = dir.path().join("keep.dat");
        let keep2 = dir.path().join("important.txt");
        fs::write(&keep, b"data").unwrap();
        fs::write(&keep2, b"data").unwrap();

        cleanup_tmp_files(dir.path()).unwrap();

        assert!(keep.exists());
        assert!(keep2.exists());
    }

    #[test]
    fn run_assembly_task_out_of_bounds_file_idx() {
        let dir = tempfile::tempdir().unwrap();
        let params = AssemblyTaskParams {
            file_idx: 5,
            tmp_dir_idx: 0,
            all_files: Arc::new(vec![]),
            all_tmp_dirs: Arc::new(vec![dir.path().to_path_buf()]),
            game_dir: dir.path().to_path_buf(),
            chunks_dir: Arc::new(dir.path().to_path_buf()),
            chunk_refcounts: Arc::new(DashMap::new()),
            verify_cache: Arc::new(DashMap::new()),
            assembled_files: Arc::new(AtomicU64::new(0)),
            last_assembly_update: Arc::new(Mutex::new(Instant::now())),
            total_files: 0,
        };

        let result = run_assembly_task(params, |_| {});
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SophonError::IndexOutOfBounds { kind: "file", .. }
        ));
    }

    #[test]
    fn assemble_file_chunk_md5_passes() {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("game");
        let chunks_dir = dir.path().join("chunks");
        let temp_dir = dir.path().join("tmp");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(&chunks_dir).unwrap();
        fs::create_dir_all(&temp_dir).unwrap();

        let data = b"chunk with md5 check";
        make_chunk_file(&chunks_dir, "ck0", data);
        let chunk_md5 = compute_md5_hex(data);
        let file_md5 = compute_md5_hex(data);

        let chunk = SophonManifestAssetChunk {
            chunk_name: "ck0".to_string(),
            chunk_decompressed_hash_md5: chunk_md5,
            chunk_on_file_offset: 0,
            chunk_size: 0,
            chunk_size_decompressed: data.len() as u64,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: String::new(),
            chunk_old_offset: -1,
        };

        let file = SophonManifestAssetProperty {
            asset_name: "verified.bin".to_string(),
            asset_chunks: vec![chunk],
            asset_type: 0,
            asset_size: data.len() as u64,
            asset_hash_md5: file_md5,
        };

        let chunk_refcounts = DashMap::new();
        chunk_refcounts.insert("ck0".to_string(), 1);
        let verify_cache = DashMap::new();

        assemble_file(
            &file,
            &game_dir,
            &chunks_dir,
            &temp_dir,
            &chunk_refcounts,
            &verify_cache,
        )
        .unwrap();

        let result = fs::read(game_dir.join("verified.bin")).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn assemble_file_chunk_md5_mismatch_fails() {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("game");
        let chunks_dir = dir.path().join("chunks");
        let temp_dir = dir.path().join("tmp");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(&chunks_dir).unwrap();
        fs::create_dir_all(&temp_dir).unwrap();

        let data = b"real chunk data here";
        make_chunk_file(&chunks_dir, "ck1", data);
        let file_md5 = compute_md5_hex(data);

        let chunk = SophonManifestAssetChunk {
            chunk_name: "ck1".to_string(),
            chunk_decompressed_hash_md5: "deadbeefdeadbeefdeadbeefdeadbeef".to_string(),
            chunk_on_file_offset: 0,
            chunk_size: 0,
            chunk_size_decompressed: data.len() as u64,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: String::new(),
            chunk_old_offset: -1,
        };

        let file = SophonManifestAssetProperty {
            asset_name: "bad.bin".to_string(),
            asset_chunks: vec![chunk],
            asset_type: 0,
            asset_size: data.len() as u64,
            asset_hash_md5: file_md5,
        };

        let chunk_refcounts = DashMap::new();
        chunk_refcounts.insert("ck1".to_string(), 1);
        let verify_cache = DashMap::new();

        let result = assemble_file(
            &file,
            &game_dir,
            &chunks_dir,
            &temp_dir,
            &chunk_refcounts,
            &verify_cache,
        );

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SophonError::Md5Mismatch { .. }
        ));
    }

    #[test]
    fn run_assembly_task_out_of_bounds_tmp_dir_idx() {
        let dir = tempfile::tempdir().unwrap();
        let file = SophonManifestAssetProperty {
            asset_name: "dummy".to_string(),
            asset_chunks: vec![],
            asset_type: 0,
            asset_size: 0,
            asset_hash_md5: String::new(),
        };
        let params = AssemblyTaskParams {
            file_idx: 0,
            tmp_dir_idx: 99,
            all_files: Arc::new(vec![file]),
            all_tmp_dirs: Arc::new(vec![]),
            game_dir: dir.path().to_path_buf(),
            chunks_dir: Arc::new(dir.path().to_path_buf()),
            chunk_refcounts: Arc::new(DashMap::new()),
            verify_cache: Arc::new(DashMap::new()),
            assembled_files: Arc::new(AtomicU64::new(0)),
            last_assembly_update: Arc::new(Mutex::new(Instant::now())),
            total_files: 1,
        };

        let result = run_assembly_task(params, |_| {});
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SophonError::IndexOutOfBounds {
                kind: "temp dir",
                ..
            }
        ));
    }

    #[test]
    fn validate_chunk_name_valid() {
        assert!(validate_chunk_name("abc123"));
    }

    #[test]
    fn validate_chunk_name_empty() {
        assert!(!validate_chunk_name(""));
    }

    #[test]
    fn validate_chunk_name_null_byte() {
        assert!(!validate_chunk_name("abc\0def"));
    }

    #[test]
    fn validate_chunk_name_absolute_path() {
        assert!(!validate_chunk_name("/etc/passwd"));
    }

    // --- Group 6: Additional chunk name security and acceptance tests ---

    /// Double-dot as a path component must be rejected (e.g. `foo/../bar`).
    /// Consecutive dots within a filename component are allowed (e.g.
    /// `foo..bar`).
    #[test]
    fn validate_chunk_name_rejects_double_dot_component() {
        assert!(!validate_chunk_name("../etc/passwd"));
        assert!(!validate_chunk_name("foo/../bar"));
        assert!(!validate_chunk_name("a/.."));
        assert!(!validate_chunk_name("a\\..\\b"));
    }

    #[test]
    fn validate_chunk_name_allows_consecutive_dots_in_filename() {
        assert!(validate_chunk_name("foo..bar"));
        assert!(validate_chunk_name("a..b"));
        assert!(validate_chunk_name("2.0..hotfix.pak"));
    }

    /// Backslash-prefixed names (Windows-style absolute paths) must be
    /// rejected.
    #[test]
    fn validate_chunk_name_rejects_backslash_prefix() {
        assert!(!validate_chunk_name("\\Windows\\System32"));
        assert!(!validate_chunk_name("\\etc\\passwd"));
    }

    /// Drive-letter style strings (e.g. C:\...) must be rejected.
    #[test]
    fn validate_chunk_name_rejects_drive_letter() {
        assert!(!validate_chunk_name("C:\\Windows"));
        assert!(!validate_chunk_name("Z:\\"));
    }

    /// Alphanumeric chunk names with underscores, hyphens and dots are valid.
    #[test]
    fn validate_chunk_name_accepts_valid_special_chars() {
        assert!(validate_chunk_name("chunk_001"));
        assert!(validate_chunk_name("chunk-v2"));
        assert!(validate_chunk_name("chunk_1.2.3"));
        assert!(validate_chunk_name("my_chunk-abc.xyz"));
    }

    /// Purely numeric chunk names (common for indexed chunks) must be accepted.
    #[test]
    fn validate_chunk_name_accepts_numeric() {
        assert!(validate_chunk_name("12345"));
        assert!(validate_chunk_name("0"));
    }

    #[test]
    fn hash_writer_writes_data_and_updates_hasher() {
        use md5::{Digest, Md5};
        let mut hasher = Md5::new();
        let mut output = Vec::new();
        {
            let mut hw = HashWriter {
                inner: &mut output,
                hasher: &mut hasher,
            };
            hw.write_all(b"hello ").unwrap();
            hw.write_all(b"world").unwrap();
            hw.flush().unwrap();
        }
        assert_eq!(output, b"hello world");
        let expected = hex::encode(Md5::digest(b"hello world"));
        assert_eq!(hex::encode(hasher.finalize()), expected);
    }

    #[test]
    fn hash_writer_empty_write() {
        use md5::{Digest, Md5};
        let mut hasher = Md5::new();
        let mut output = Vec::new();
        {
            let mut hw = HashWriter {
                inner: &mut output,
                hasher: &mut hasher,
            };
            hw.write_all(b"").unwrap();
        }
        assert!(output.is_empty());
        let expected = hex::encode(Md5::digest(b""));
        assert_eq!(hex::encode(hasher.finalize()), expected);
    }

    #[test]
    fn hash_writer_multiple_writes_accumulate_hash() {
        use md5::{Digest, Md5};
        let mut hasher = Md5::new();
        let mut output = Vec::new();
        {
            let mut hw = HashWriter {
                inner: &mut output,
                hasher: &mut hasher,
            };
            hw.write_all(b"a").unwrap();
            hw.write_all(b"b").unwrap();
            hw.write_all(b"c").unwrap();
        }
        let combined_hash = hex::encode(Md5::digest(b"abc"));
        assert_eq!(hex::encode(hasher.finalize()), combined_hash);
    }

    /// Test write_from_old_file reads from correct offset and verifies hash
    #[test]
    fn write_from_old_file_reads_correct_offset() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let old_file_path = dir.path().join("old_file.bin");

        // Create old file with known content: "AAAA" at offset 0, "BBBB" at offset 4
        let mut old_file = fs::File::create(&old_file_path).unwrap();
        old_file.write_all(b"AAAABBBB").unwrap();
        drop(old_file);

        // Create output file
        let output_path = dir.path().join("output.bin");
        let mut output_file = fs::File::create(&output_path).unwrap();
        let mut writer = std::io::BufWriter::new(&mut output_file);

        // Read 4 bytes from offset 4 (should get "BBBB")
        let mut transfer_buf = vec![0u8; 1024];
        let bytes_written = write_from_old_file(
            &old_file_path,
            &mut writer,
            0,    // new_offset
            4,    // old_offset
            4,    // expected_size
            None, // file_hasher
            &mut transfer_buf,
            "", // chunk_decompressed_hash_md5 (skip verification)
        )
        .unwrap();

        writer.flush().unwrap();
        drop(writer);
        drop(output_file);

        assert_eq!(bytes_written, 4);
        let result = fs::read(&output_path).unwrap();
        assert_eq!(&result, b"BBBB");
    }

    /// Test write_from_old_file verifies chunk hash correctly
    #[test]
    fn write_from_old_file_verifies_chunk_hash() {
        use md5::{Digest, Md5};

        let dir = tempfile::tempdir().unwrap();
        let old_file_path = dir.path().join("old_file.bin");

        let data = b"test data for hash verification";
        let expected_md5 = hex::encode(Md5::digest(data));

        let mut old_file = fs::File::create(&old_file_path).unwrap();
        old_file.write_all(data).unwrap();
        drop(old_file);

        let output_path = dir.path().join("output.bin");
        let mut output_file = fs::File::create(&output_path).unwrap();
        let mut writer = std::io::BufWriter::new(&mut output_file);

        let mut transfer_buf = vec![0u8; 1024];
        let result = write_from_old_file(
            &old_file_path,
            &mut writer,
            0,
            0,
            data.len() as u64,
            None,
            &mut transfer_buf,
            &expected_md5,
        );

        assert!(result.is_ok(), "should succeed with correct hash");
    }

    /// Test write_from_old_file fails on hash mismatch
    #[test]
    fn write_from_old_file_hash_mismatch_fails() {
        let dir = tempfile::tempdir().unwrap();
        let old_file_path = dir.path().join("old_file.bin");

        let data = b"test data";
        let wrong_md5 = "ffffffffffffffffffffffffffffffff"; // Not EMPTY_MD5

        let mut old_file = fs::File::create(&old_file_path).unwrap();
        old_file.write_all(data).unwrap();
        drop(old_file);

        let output_path = dir.path().join("output.bin");
        let mut output_file = fs::File::create(&output_path).unwrap();
        let mut writer = std::io::BufWriter::new(&mut output_file);

        let mut transfer_buf = vec![0u8; 1024];
        let result = write_from_old_file(
            &old_file_path,
            &mut writer,
            0,
            0,
            data.len() as u64,
            None,
            &mut transfer_buf,
            wrong_md5,
        );

        assert!(result.is_err(), "should fail with wrong hash");
        assert!(matches!(
            result.unwrap_err(),
            SophonError::Md5Mismatch { .. }
        ));
    }

    /// Test write_from_old_file fails when old file is too short
    #[test]
    fn write_from_old_file_too_short_fails() {
        let dir = tempfile::tempdir().unwrap();
        let old_file_path = dir.path().join("old_file.bin");

        // Create file with only 5 bytes
        let data = b"short";
        let mut old_file = fs::File::create(&old_file_path).unwrap();
        old_file.write_all(data).unwrap();
        drop(old_file);

        let output_path = dir.path().join("output.bin");
        let mut output_file = fs::File::create(&output_path).unwrap();
        let mut writer = std::io::BufWriter::new(&mut output_file);

        let mut transfer_buf = vec![0u8; 1024];
        // Try to read 10 bytes from a 5-byte file
        let result = write_from_old_file(
            &old_file_path,
            &mut writer,
            0,
            0,
            10, // expected_size > actual size
            None,
            &mut transfer_buf,
            "",
        );

        assert!(result.is_err(), "should fail when file is too short");
    }

    /// Test write_from_old_file fails when old file doesn't exist
    #[test]
    fn write_from_old_file_missing_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        let old_file_path = dir.path().join("nonexistent.bin");

        let output_path = dir.path().join("output.bin");
        let mut output_file = fs::File::create(&output_path).unwrap();
        let mut writer = std::io::BufWriter::new(&mut output_file);

        let mut transfer_buf = vec![0u8; 1024];
        let result = write_from_old_file(
            &old_file_path,
            &mut writer,
            0,
            0,
            100,
            None,
            &mut transfer_buf,
            "",
        );

        assert!(result.is_err(), "should fail when old file doesn't exist");
    }

    /// Test assemble_file with chunk_old_offset reuses data from old file
    #[test]
    fn assemble_file_reuses_chunk_from_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("game");
        let chunks_dir = dir.path().join("chunks");
        let temp_dir = dir.path().join("tmp");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(&chunks_dir).unwrap();
        fs::create_dir_all(&temp_dir).unwrap();

        // Create old file with content that will be reused
        let old_data = b"reused chunk data here!";
        let target_path = game_dir.join("reused.bin");
        fs::write(&target_path, old_data).unwrap();

        let md5 = compute_md5_hex(old_data);

        // Create asset with chunk_old_offset >= 0 (reuse from old file)
        let file = SophonManifestAssetProperty {
            asset_name: "reused.bin".to_string(),
            asset_chunks: vec![SophonManifestAssetChunk {
                chunk_name: "not_used".to_string(), // Won't be used due to old_offset
                chunk_decompressed_hash_md5: md5.clone(),
                chunk_on_file_offset: 0,
                chunk_size: 0,
                chunk_size_decompressed: old_data.len() as u64,
                chunk_compressed_hash_xxh: 0,
                chunk_compressed_hash_md5: String::new(),
                chunk_old_offset: 0, // Reuse from old file at offset 0
            }],
            asset_type: 0,
            asset_size: old_data.len() as u64,
            asset_hash_md5: md5,
        };

        let chunk_refcounts = DashMap::new();
        let verify_cache = DashMap::new();

        // This should reuse data from the existing file, not fail due to missing chunk
        let result = assemble_file(
            &file,
            &game_dir,
            &chunks_dir,
            &temp_dir,
            &chunk_refcounts,
            &verify_cache,
        );

        assert!(
            result.is_ok(),
            "should succeed with chunk reuse: {:?}",
            result.err()
        );

        // Verify the file still has the correct content
        let result_data = fs::read(&target_path).unwrap();
        assert_eq!(&result_data, old_data);

        // Verify no chunk was downloaded (refcount should still be 0)
        assert!(!chunk_refcounts.contains_key("not_used"));
    }
}
