use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use md5::{Digest, Md5};
use tauri_plugin_log::log;

use super::cache::VerificationEntry;
use super::error::{SophonError, SophonResult};
use super::{FILE_WRITE_BUFFER_SIZE, PROGRESS_UPDATE_INTERVAL_MS};
use crate::commands::sophon_downloader::SophonProgress;
use crate::commands::sophon_downloader::proto_parse::{
    SophonManifestAssetChunk, SophonManifestAssetProperty,
};

#[inline]
pub fn chunk_filename(chunk: &SophonManifestAssetChunk) -> String {
    format!("{}.zstd", chunk.chunk_name)
}

#[inline]
pub fn decrement_chunk_refcount(
    chunk_name: &str,
    chunk_refcounts: &DashMap<String, usize>,
    chunks_dir: &Path,
) {
    if let Some(mut count) = chunk_refcounts.get_mut(chunk_name) {
        *count -= 1;
        if *count == 0 {
            drop(count);
            chunk_refcounts.remove(chunk_name);
            let _ = fs::remove_file(chunks_dir.join(format!("{}.zstd", chunk_name)));
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
        } else if path.extension().map(|e| e == "tmp").unwrap_or(false) {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
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
    if name.contains("..") {
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

pub fn assemble_file(
    file: &SophonManifestAssetProperty,
    game_dir: &Path,
    chunks_dir: &Path,
    temp_dir: &Path,
    chunk_refcounts: &DashMap<String, usize>,
    verify_cache: &DashMap<String, VerificationEntry>,
) -> SophonResult<()> {
    validate_asset_name(&file.asset_name)?;
    let target_path = game_dir.join(&file.asset_name);
    let tmp_path = temp_dir.join(format!(
        "{}.tmp",
        file.asset_name.replace(['/', '\\', ':'], "_")
    ));

    if target_path.exists() {
        let already_valid = super::cache::check_file_md5_cached(
            &target_path,
            file.asset_size,
            &file.asset_hash_md5,
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

    if tmp_path.exists() {
        fs::remove_file(&tmp_path)?;
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

    use std::io::Write;
    {
        let mut buf_writer_pre = BufWriter::with_capacity(FILE_WRITE_BUFFER_SIZE, &out_file);
        let zeros = vec![0u8; FILE_WRITE_BUFFER_SIZE];
        let mut remaining = file.asset_size;
        while remaining > 0 {
            let to_write = (remaining as usize).min(FILE_WRITE_BUFFER_SIZE);
            buf_writer_pre.write_all(&zeros[..to_write])?;
            remaining -= to_write as u64;
        }
        buf_writer_pre.flush()?;
    }

    let mut buf_writer = BufWriter::with_capacity(FILE_WRITE_BUFFER_SIZE, out_file);
    let mut total_written: u64 = 0;
    let mut file_hasher = if file.asset_hash_md5.is_empty() {
        None
    } else {
        Some(Md5::new())
    };

    for chunk in &file.asset_chunks {
        let chunk_path = chunks_dir.join(chunk_filename(chunk));

        let bytes_written = write_decompressed_chunk_at(
            &chunk_path,
            &mut buf_writer,
            chunk.chunk_on_file_offset,
            chunk.chunk_size_decompressed,
            file_hasher.as_mut(),
        )?;

        total_written += bytes_written;

        decrement_chunk_refcount(&chunk.chunk_name, chunk_refcounts, chunks_dir);
    }

    buf_writer.flush()?;
    let out_file = buf_writer
        .into_inner()
        .map_err(|e| SophonError::Io(e.into_error()))?;
    out_file.sync_data()?;

    if total_written != file.asset_size {
        return Err(SophonError::SizeMismatch {
            item: file.asset_name.clone(),
            expected: file.asset_size,
            actual: total_written,
        });
    }

    if let Some(hasher) = file_hasher {
        let actual = hex::encode(hasher.finalize());
        if actual != file.asset_hash_md5 {
            return Err(SophonError::Md5Mismatch {
                item: file.asset_name.clone(),
                expected: file.asset_hash_md5.clone(),
                actual,
            });
        }
    }

    fs::rename(&tmp_path, &target_path)?;
    Ok(())
}

fn write_decompressed_chunk_at<W: Write + Seek>(
    chunk_path: &Path,
    writer: &mut W,
    offset: u64,
    expected_size: u64,
    file_hasher: Option<&mut Md5>,
) -> SophonResult<u64> {
    let f = File::open(chunk_path)?;
    let mut decoder = zstd::Decoder::new(f)?;

    writer.flush()?;
    writer.seek(SeekFrom::Start(offset))?;

    let bytes_written = match file_hasher {
        Some(hasher) => {
            let mut hw = HashWriter {
                inner: writer,
                hasher,
            };
            std::io::copy(&mut decoder, &mut hw)?
        }
        None => std::io::copy(&mut decoder, writer)?,
    };

    if bytes_written != expected_size {
        return Err(SophonError::SizeMismatch {
            item: chunk_path.display().to_string(),
            expected: expected_size,
            actual: bytes_written,
        });
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
    .map_err(|e| SophonError::AssemblyFailed {
        file: file.asset_name.clone(),
        error: e.to_string(),
    })?;

    let count = assembled_files.fetch_add(1, Ordering::Relaxed) + 1;

    if count % 50 == 0 {
        let gd = game_dir.clone();
        let vc = verify_cache.clone();
        std::thread::spawn(move || {
            if let Err(e) = super::cache::save_verification_cache(&gd, &vc) {
                log::error!("Periodic verification cache save failed: {}", e);
            }
        });
    }

    {
        let mut lu = last_assembly_update.lock().unwrap_or_else(|e| {
            log::error!("last_assembly_update mutex poisoned, recovering");
            e.into_inner()
        });
        if lu.elapsed() >= Duration::from_millis(PROGRESS_UPDATE_INTERVAL_MS) {
            updater(SophonProgress::Assembling {
                assembled_files: count,
                total_files,
            });
            *lu = Instant::now();
        }
    }

    Ok(())
}

pub fn spawn_assembly_task(
    params: AssemblyTaskParams,
    updater: impl Fn(SophonProgress) + Send + Sync + 'static,
) -> tokio::task::JoinHandle<SophonResult<()>> {
    tokio::task::spawn_blocking(move || run_assembly_task(params, updater))
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
    fn chunk_filename_format() {
        let chunk = SophonManifestAssetChunk {
            chunk_name: "abc123".into(),
            chunk_decompressed_hash_md5: String::new(),
            chunk_on_file_offset: 0,
            chunk_size: 0,
            chunk_size_decompressed: 0,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: String::new(),
        };
        assert_eq!(chunk_filename(&chunk), "abc123.zstd");
    }

    fn make_chunk_file(chunks_dir: &Path, chunk_name: &str, data: &[u8]) {
        let compressed = zstd::encode_all(data, 0).unwrap();
        fs::write(chunks_dir.join(format!("{}.zstd", chunk_name)), &compressed).unwrap();
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
}
