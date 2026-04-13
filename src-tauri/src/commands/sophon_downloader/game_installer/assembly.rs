use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use md5::{Digest, Md5};

use super::cache::VerificationEntry;
use super::constants::{
    DECOMPRESSION_BUFFER_SIZE, FILE_WRITE_BUFFER_SIZE, PROGRESS_UPDATE_INTERVAL_MS,
};
use super::manifest::SophonManifestAssetChunk;
use crate::commands::sophon_downloader::SophonProgress;
use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetProperty;

pub fn chunk_filename(chunk: &SophonManifestAssetChunk) -> String {
    format!("{}.zstd", chunk.chunk_name)
}

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

pub fn validate_asset_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("asset_name cannot be empty".to_string());
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return Err(format!("asset_name cannot be absolute path: {}", name));
    }
    if name.contains("..") {
        return Err(format!("asset_name cannot contain '..': {}", name));
    }
    if name.contains('\0') {
        return Err("asset_name cannot contain null bytes".to_string());
    }
    let mut chars = name.chars();
    if let (Some(first), Some(':')) = (chars.next(), chars.next()) {
        if first.is_ascii_alphabetic() {
            return Err(format!("asset_name cannot contain drive letters: {}", name));
        }
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
) -> Result<(), Box<dyn std::error::Error>> {
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
            for chunk in &file.asset_chunks {
                decrement_chunk_refcount(&chunk.chunk_name, chunk_refcounts, chunks_dir);
            }
            return Ok(());
        }
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
        .open(&tmp_path)?;
    out_file.set_len(file.asset_size)?;

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
    let out_file = buf_writer.into_inner().map_err(|e| e.into_error())?;
    out_file.sync_data()?;

    if total_written != file.asset_size {
        return Err(format!(
            "File {} total written {} != expected {}",
            file.asset_name, total_written, file.asset_size
        )
        .into());
    }

    if let Some(hasher) = file_hasher {
        let actual = format!("{:x}", hasher.finalize());
        if actual != file.asset_hash_md5 {
            return Err(format!(
                "Final file MD5 mismatch for {}: expected {}, got {}",
                file.asset_name, file.asset_hash_md5, actual
            )
            .into());
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
    mut file_hasher: Option<&mut Md5>,
) -> Result<u64, Box<dyn std::error::Error>> {
    let f = File::open(chunk_path)?;
    let mut decoder = zstd::Decoder::new(f)?;
    let mut total_written = 0u64;
    let mut buf = vec![0u8; DECOMPRESSION_BUFFER_SIZE];

    writer.seek(SeekFrom::Start(offset))?;

    loop {
        let n = decoder.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if let Some(hasher) = file_hasher.as_mut() {
            hasher.update(&buf[..n]);
        }
        writer.write_all(&buf[..n])?;
        total_written += n as u64;
    }

    if total_written != expected_size {
        return Err(format!(
            "Decompressed size mismatch: expected {}, got {}",
            expected_size, total_written
        )
        .into());
    }

    Ok(total_written)
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
) -> Result<(), String> {
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
        return Err(format!("file index {} out of bounds", file_idx));
    }
    if tmp_dir_idx >= all_tmp_dirs.len() {
        return Err(format!("tmp_dir index {} out of bounds", tmp_dir_idx));
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
    .map_err(|e| format!("Failed to assemble {}: {e}", file.asset_name))?;

    let count = assembled_files.fetch_add(1, Ordering::Relaxed) + 1;

    {
        let mut lu = last_assembly_update.lock().unwrap();
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
) -> tokio::task::JoinHandle<Result<(), String>> {
    tokio::task::spawn_blocking(move || run_assembly_task(params, updater))
}
