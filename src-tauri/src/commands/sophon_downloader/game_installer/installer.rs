use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::{DashMap, DashSet};
use futures_util::future::try_join_all;
use reqwest::Client;
use sha2::{Digest, Sha256};
use tauri_plugin_log::log;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::adaptive_assembly::AdaptiveAssembly;
use super::adaptive_download::AdaptiveSemaphore;
use super::api::{fetch_build, fetch_front_door, is_known_vo_locale, vo_lang_matches};
use super::assembly::{
    self, AssemblyTaskParams, cleanup_tmp_files, spawn_assembly_task, validate_asset_name,
    validate_chunk_name,
};
use super::cache::{self, VerificationEntry};
use super::error::{SophonError, SophonResult};
use super::handle::DownloadHandle;
use super::*;
use crate::commands::sophon_downloader::SophonProgress;
use crate::commands::sophon_downloader::api_scrape::{
    DownloadInfo, SophonBuildData, SophonManifestMeta,
};
use crate::commands::sophon_downloader::proto_parse::{
    SophonManifestAssetChunk, SophonManifestAssetProperty, SophonManifestProto,
};

type ProgressUpdater = Arc<dyn Fn(SophonProgress) + Send + Sync>;
pub type StateSaver = Arc<dyn Fn(&DashMap<String, u64>) + Send + Sync>;

pub struct ResumeContext {
    pub prev_manifest_hash: String,
    pub prev_downloaded_chunks: HashMap<String, u64>,
}

struct InstallContext {
    chunks_dir: Arc<PathBuf>,
    game_dir: PathBuf,
    all_tmp_dirs: Arc<Vec<std::path::PathBuf>>,
    all_files: Arc<Vec<SophonManifestAssetProperty>>,
    downloaded_bytes: Arc<AtomicU64>,
    assembled_files: Arc<AtomicU64>,
    total_bytes: u64,
    total_files: u64,
    resume_bytes_offset: Arc<AtomicU64>,
    verify_cache: Arc<DashMap<String, VerificationEntry>>,
    chunk_refcounts: Arc<DashMap<String, usize>>,
    last_assembly_update: Arc<Mutex<Instant>>,
    last_update: Arc<AtomicU64>,
    /// EWMA-smoothed speed for display (bytes/sec, scaled by 1000).
    smooth_speed_bps: Arc<AtomicU64>,
    /// Ring buffer of recent speed samples for ETA calculation.
    eta_speed_history: Arc<Mutex<VecDeque<f64>>>,
    last_speed_bytes: Arc<AtomicU64>,
    last_speed_time: Arc<AtomicU64>,
    updater: ProgressUpdater,
    downloaded_chunks: Arc<DashMap<String, u64>>,
    chunks_since_save: Arc<AtomicU64>,
    last_save: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    state_saver: StateSaver,
    adaptive_assembly: Arc<AdaptiveAssembly>,
}

static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

#[inline]
fn now_nanos() -> u64 {
    EPOCH.elapsed().as_nanos() as u64
}

pub(crate) struct InstallerData {
    client: Arc<Client>,
    chunk_download: Arc<DownloadInfo>,
    file_count: usize,
    label: String,
    pub matching_field: String,
}

struct DownloadItem {
    file_idx: usize,
    chunk_idx: usize,
    client: Arc<Client>,
    chunk_download: Arc<DownloadInfo>,
    is_pre_downloaded: bool,
}

type PendingCount = Arc<AtomicUsize>;
type FileEntry = (usize, usize, PendingCount);

pub struct SophonInstaller {
    pub client: Client,
    pub manifest: SophonManifestProto,
    pub chunk_download: DownloadInfo,
    pub label: String,
    pub matching_field: String,
    pub manifest_hash: String,
}

impl SophonInstaller {
    pub async fn from_manifest_meta(
        client: &Client,
        meta: &SophonManifestMeta,
    ) -> SophonResult<Self> {
        let result =
            super::api::fetch_manifest(client, &meta.manifest_download, &meta.manifest.id).await?;
        let mut manifest = result.manifest;
        // Fresh install: all chunks must be downloaded (no old-file reuse)
        for asset in &mut manifest.assets {
            for chunk in &mut asset.asset_chunks {
                chunk.chunk_old_offset = -1;
            }
        }
        Ok(Self {
            client: client.clone(),
            manifest,
            chunk_download: meta.chunk_download.clone(),
            label: meta
                .chunk_download
                .url_suffix
                .trim_matches('/')
                .replace('/', "-"),
            matching_field: meta.matching_field.clone(),
            manifest_hash: result.hash,
        })
    }
}

pub async fn build_installers(
    client: &Client,
    game_id: &str,
    vo_lang: &str,
) -> SophonResult<(Vec<SophonInstaller>, String, String)> {
    let (branch, _) = fetch_front_door(client, game_id).await?;

    let build = fetch_build(
        client,
        branch.main.as_ref().ok_or(SophonError::NoGameManifest)?,
        None,
    )
    .await?;
    let tag = build.tag.clone();

    let installers = build_installers_from_data(client, &build, vo_lang).await?;
    let manifest_hash = combine_manifest_hashes(&installers);
    Ok((installers, tag, manifest_hash))
}

pub async fn build_update_installers(
    client: &Client,
    game_id: &str,
    vo_lang: &str,
    from_tag: &str,
) -> SophonResult<(Vec<SophonInstaller>, Vec<String>, String, String)> {
    let (branch, _) = fetch_front_door(client, game_id).await?;

    let (old_build, new_build) = tokio::try_join!(
        fetch_build(
            client,
            branch.main.as_ref().ok_or(SophonError::NoGameManifest)?,
            Some(from_tag)
        ),
        fetch_build(
            client,
            branch.main.as_ref().ok_or(SophonError::NoGameManifest)?,
            None
        ),
    )?;

    let new_tag = new_build.tag.clone();
    let (installers, deleted_files) =
        build_diff_installers(client, &old_build, &new_build, vo_lang).await?;
    let manifest_hash = combine_manifest_hashes(&installers);
    Ok((installers, deleted_files, new_tag, manifest_hash))
}

async fn build_installers_from_data(
    client: &Client,
    build: &SophonBuildData,
    vo_lang: &str,
) -> SophonResult<Vec<SophonInstaller>> {
    let qualifying: Vec<&SophonManifestMeta> = build
        .manifests
        .iter()
        .filter(|m| {
            m.matching_field == "game"
                || vo_lang_matches(&m.matching_field, vo_lang)
                || !is_known_vo_locale(&m.matching_field)
        })
        .collect();

    if qualifying.is_empty() {
        return Err(SophonError::NoGameManifest);
    }

    let futures: Vec<_> = qualifying
        .iter()
        .map(|meta| SophonInstaller::from_manifest_meta(client, meta))
        .collect();
    let installers = try_join_all(futures).await?;

    Ok(installers)
}

fn combine_manifest_hashes(installers: &[SophonInstaller]) -> String {
    let mut hashes: Vec<&str> = installers
        .iter()
        .map(|i| i.manifest_hash.as_str())
        .collect();
    hashes.sort();
    let mut hasher = Sha256::new();
    for h in hashes {
        hasher.update(h.as_bytes());
    }
    hex::encode(&hasher.finalize()[..8])
}

#[inline]
fn collect_deleted_files(
    old_manifest: &SophonManifestProto,
    new_names: &HashSet<&str>,
) -> Vec<String> {
    old_manifest
        .assets
        .iter()
        .filter(|f| !f.is_directory() && !new_names.contains(f.asset_name.as_str()))
        .map(|f| f.asset_name.clone())
        .collect()
}

#[inline]
fn build_old_md5_map(old_manifest: SophonManifestProto) -> HashMap<String, String> {
    old_manifest
        .assets
        .into_iter()
        .filter(|f| !f.is_directory())
        .map(|f| (f.asset_name, f.asset_hash_md5))
        .collect()
}

#[inline]
fn compute_diff_files(
    new_manifest: SophonManifestProto,
    old_md5_map: &HashMap<String, String>,
) -> Vec<SophonManifestAssetProperty> {
    new_manifest
        .assets
        .into_iter()
        .filter(|f| {
            if f.is_directory() {
                return true;
            }
            match old_md5_map.get(&f.asset_name) {
                Some(old_md5) => old_md5 != &f.asset_hash_md5,
                None => true,
            }
        })
        .collect()
}

async fn build_diff_installers(
    client: &Client,
    old_build: &SophonBuildData,
    new_build: &SophonBuildData,
    vo_lang: &str,
) -> SophonResult<(Vec<SophonInstaller>, Vec<String>)> {
    let old_by_field: HashMap<&str, &SophonManifestMeta> = old_build
        .manifests
        .iter()
        .map(|m| (m.matching_field.as_str(), m))
        .collect();

    let mut installers = Vec::with_capacity(new_build.manifests.len());
    let mut deleted_files: Vec<String> = Vec::new();

    for new_meta in &new_build.manifests {
        if new_meta.matching_field != "game"
            && !vo_lang_matches(&new_meta.matching_field, vo_lang)
            && is_known_vo_locale(&new_meta.matching_field)
        {
            continue;
        }

        let new_result =
            super::api::fetch_manifest(client, &new_meta.manifest_download, &new_meta.manifest.id)
                .await?;
        let new_manifest_hash = new_result.hash.clone();

        let new_names: HashSet<&str> = new_result
            .manifest
            .assets
            .iter()
            .map(|f| f.asset_name.as_str())
            .collect();

        let (old_md5_map, old_chunk_offsets): (
            HashMap<String, String>,
            HashMap<(String, String), u64>,
        ) = match old_by_field.get(new_meta.matching_field.as_str()) {
            Some(old_meta) => {
                let old_result = super::api::fetch_manifest(
                    client,
                    &old_meta.manifest_download,
                    &old_meta.manifest.id,
                )
                .await?;

                deleted_files.extend(collect_deleted_files(&old_result.manifest, &new_names));

                // Build old-chunk (asset_name, hash) → offset map BEFORE
                // consuming old_result.manifest. Keying by (asset_name, hash)
                // preserves file provenance: when a chunk's decompressed hash
                // matches an old chunk, the offset must come from the same old
                // source file that the new file will reuse.
                let old_chunk_offsets: HashMap<(String, String), u64> = old_result
                    .manifest
                    .assets
                    .iter()
                    .filter(|f| !f.is_directory())
                    .flat_map(|f| {
                        let name = f.asset_name.clone();
                        f.asset_chunks.iter().map(move |c| {
                            (
                                (name.clone(), c.chunk_decompressed_hash_md5.clone()),
                                c.chunk_on_file_offset,
                            )
                        })
                    })
                    .collect();

                let old_md5_map = build_old_md5_map(old_result.manifest);
                (old_md5_map, old_chunk_offsets)
            }
            None => (HashMap::new(), HashMap::new()),
        };

        let mut diff_files = compute_diff_files(new_result.manifest, &old_md5_map);

        // Chunk-level diff: annotate each chunk with the old-file offset only
        // when the corresponding old chunk belonged to the same asset. Chunks
        // introduced in the new version (no entry in the map) are marked -1 so
        // they get downloaded.
        for file in &mut diff_files {
            for chunk in &mut file.asset_chunks {
                let key = (
                    file.asset_name.clone(),
                    chunk.chunk_decompressed_hash_md5.clone(),
                );
                chunk.chunk_old_offset = old_chunk_offsets
                    .get(&key)
                    .map(|&off| off as i64)
                    .unwrap_or(-1);
            }
        }

        if diff_files.is_empty() {
            continue;
        }

        installers.push(SophonInstaller {
            client: client.clone(),
            manifest: SophonManifestProto { assets: diff_files },
            chunk_download: new_meta.chunk_download.clone(),
            label: new_meta
                .chunk_download
                .url_suffix
                .trim_matches('/')
                .replace('/', "-"),
            matching_field: new_meta.matching_field.clone(),
            manifest_hash: new_manifest_hash,
        });
    }

    Ok((installers, deleted_files))
}

async fn prepare_directories(game_dir: &Path, chunks_dir: &Path) -> SophonResult<()> {
    let cd = chunks_dir.to_path_buf();
    tokio::task::spawn_blocking(move || fs::create_dir_all(&cd))
        .await?
        .map_err(SophonError::from)?;

    let gd = game_dir.to_path_buf();
    tokio::task::spawn_blocking(move || cleanup_tmp_files(&gd))
        .await?
        .map_err(SophonError::from)?;

    Ok(())
}

fn build_installer_data(
    installers: Vec<SophonInstaller>,
) -> (Vec<InstallerData>, Vec<SophonManifestAssetProperty>) {
    let mut all_files = Vec::new();
    let mut data = Vec::with_capacity(installers.len());
    for inst in installers {
        let file_count = inst
            .manifest
            .assets
            .iter()
            .filter(|a| !a.is_directory())
            .count();
        all_files.extend(
            inst.manifest
                .assets
                .into_iter()
                .filter(|a| !a.is_directory()),
        );
        data.push(InstallerData {
            label: inst.label,
            matching_field: inst.matching_field,
            client: Arc::new(inst.client),
            chunk_download: Arc::new(inst.chunk_download),
            file_count,
        });
    }
    (data, all_files)
}

fn compute_totals(all_files: &[SophonManifestAssetProperty]) -> (u64, u64) {
    let mut seen_chunks: HashSet<&str> = HashSet::new();
    let total_compressed: u64 = all_files
        .iter()
        .flat_map(|f| f.asset_chunks.iter())
        .filter(|c| c.chunk_old_offset < 0)
        .filter(|c| seen_chunks.insert(c.chunk_name.as_str()))
        .map(|c| c.chunk_size)
        .fold(0u64, |acc, x| acc.saturating_add(x));

    let total_files: u64 = all_files.len() as u64;

    (total_compressed, total_files)
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn register_chunks_for_file(
    file: &SophonManifestAssetProperty,
    file_idx: usize,
    tmp_dir_idx: usize,
    ctx: &InstallContext,
    chunk_entries: &mut Vec<Vec<FileEntry>>,
    download_items: &mut Vec<DownloadItem>,
    download_items_index: &mut HashMap<String, usize>,
    data: &InstallerData,
    pre_downloaded: &HashSet<String>,
) {
    let downloadable: Vec<(usize, &SophonManifestAssetChunk)> = file
        .asset_chunks
        .iter()
        .enumerate()
        .filter(|(_, c)| c.chunk_old_offset < 0)
        .collect();

    let chunk_count = downloadable.len();
    if chunk_count == 0 {
        return;
    }

    let pending = Arc::new(AtomicUsize::new(chunk_count));
    for (chunk_idx, chunk) in downloadable {
        let name = chunk.chunk_name.as_str();
        let is_pre = pre_downloaded.contains(name);

        let item_idx = if let Some(&idx) = download_items_index.get(name) {
            if is_pre {
                download_items[idx].is_pre_downloaded = true;
            }
            idx
        } else {
            let idx = download_items.len();
            download_items.push(DownloadItem {
                file_idx,
                chunk_idx,
                client: Arc::clone(&data.client),
                chunk_download: Arc::clone(&data.chunk_download),
                is_pre_downloaded: is_pre,
            });
            download_items_index.insert(chunk.chunk_name.clone(), idx);
            idx
        };

        if item_idx >= chunk_entries.len() {
            chunk_entries.resize_with(item_idx + 1, Vec::new);
        }
        chunk_entries[item_idx].push((file_idx, tmp_dir_idx, Arc::clone(&pending)));

        if let Some(mut count) = ctx.chunk_refcounts.get_mut(name) {
            *count += 1;
        } else {
            ctx.chunk_refcounts.insert(name.to_owned(), 1);
        }
    }
}

async fn build_download_state(
    installer_data: Vec<InstallerData>,
    ctx: &InstallContext,
    assemble_tx: &mpsc::Sender<(usize, usize)>,
    completed_indices: Option<&HashSet<usize>>,
    pre_downloaded: &HashSet<String>,
) -> SophonResult<(Vec<DownloadItem>, Arc<Vec<Vec<FileEntry>>>)> {
    let total_chunks: usize = ctx
        .all_files
        .iter()
        .map(|f| f.asset_chunks.len())
        .fold(0usize, |acc, x| acc.saturating_add(x));
    let mut download_items: Vec<DownloadItem> = Vec::with_capacity(total_chunks);
    let mut download_items_index: HashMap<String, usize> = HashMap::with_capacity(total_chunks);
    let mut chunk_entries: Vec<Vec<FileEntry>> = Vec::with_capacity(total_chunks);

    let mut all_files_index: usize = 0;

    for (tmp_dir_idx, data) in installer_data.into_iter().enumerate() {
        let needs_tmp_dir = (0..data.file_count)
            .any(|i| completed_indices.is_none_or(|set| !set.contains(&(all_files_index + i))));
        if needs_tmp_dir {
            let tmp_dir = &ctx.all_tmp_dirs[tmp_dir_idx];
            let td = tmp_dir.clone();
            tokio::task::spawn_blocking(move || fs::create_dir_all(&td))
                .await?
                .map_err(SophonError::from)?;
        }

        for _ in 0..data.file_count {
            if completed_indices.is_some_and(|set| set.contains(&all_files_index)) {
                all_files_index += 1;
                continue;
            }

            let file = &ctx.all_files[all_files_index];
            let has_downloadable = file.asset_chunks.iter().any(|c| c.chunk_old_offset < 0);

            if !has_downloadable {
                let _ = assemble_tx.send((all_files_index, tmp_dir_idx)).await;
                all_files_index += 1;
                continue;
            }

            register_chunks_for_file(
                file,
                all_files_index,
                tmp_dir_idx,
                ctx,
                &mut chunk_entries,
                &mut download_items,
                &mut download_items_index,
                &data,
                pre_downloaded,
            );
            all_files_index += 1;
        }
    }

    log::info!(
        "build_download_state: {} download items, {} chunk entries, all_files_index={}",
        download_items.len(),
        chunk_entries.len(),
        all_files_index,
    );
    Ok((download_items, Arc::new(chunk_entries)))
}

fn make_assembly_params(
    ctx: &InstallContext,
    file_idx: usize,
    tmp_dir_idx: usize,
) -> AssemblyTaskParams {
    AssemblyTaskParams {
        file_idx,
        tmp_dir_idx,
        all_files: Arc::clone(&ctx.all_files),
        all_tmp_dirs: Arc::clone(&ctx.all_tmp_dirs),
        game_dir: ctx.game_dir.clone(),
        chunks_dir: Arc::clone(&ctx.chunks_dir),
        chunk_refcounts: Arc::clone(&ctx.chunk_refcounts),
        verify_cache: Arc::clone(&ctx.verify_cache),
        assembled_files: Arc::clone(&ctx.assembled_files),
        last_assembly_update: Arc::clone(&ctx.last_assembly_update),
        total_files: ctx.total_files,
    }
}

async fn drain_join_set(join_set: &mut tokio::task::JoinSet<SophonResult<()>>) -> SophonResult<()> {
    let mut first_error: Option<SophonError> = None;
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                log::error!("Assembly task failed: {err}");
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
            Err(err) => {
                log::error!("Assembly task join error: {err}");
                if first_error.is_none() {
                    first_error = Some(SophonError::JoinError(err));
                }
            }
        }
    }
    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

#[allow(clippy::let_and_return)]
fn spawn_assembly_coordinator(
    ctx: &Arc<InstallContext>,
    assemble_rx: mpsc::Receiver<(usize, usize)>,
    assembly_cancel: tokio_util::sync::CancellationToken,
) -> tokio::task::JoinHandle<SophonResult<()>> {
    let ctx = Arc::clone(ctx);
    ctx.adaptive_assembly
        .spawn_adjuster(assembly_cancel.clone());
    let task_cancel = assembly_cancel;

    let handle = tokio::spawn(async move {
        let mut rx = assemble_rx;
        let cancel = task_cancel;
        let mut join_set = tokio::task::JoinSet::new();

        loop {
            let max_concurrency = ctx.adaptive_assembly.current_target();
            while join_set.len() < max_concurrency {
                match rx.try_recv() {
                    Ok((file_idx, tmp_dir_idx)) => {
                        let params = make_assembly_params(&ctx, file_idx, tmp_dir_idx);
                        let updater = Arc::clone(&ctx.updater);
                        join_set.spawn(spawn_assembly_task(params, move |p| updater(p)));
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        return drain_join_set(&mut join_set).await;
                    }
                }
            }

            if join_set.is_empty() {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        return drain_join_set(&mut join_set).await;
                    }
                    msg = rx.recv() => {
                        let Some((file_idx, tmp_dir_idx)) = msg else {
                            return drain_join_set(&mut join_set).await;
                        };
                        let params = make_assembly_params(&ctx, file_idx, tmp_dir_idx);
                        let updater = Arc::clone(&ctx.updater);
                        join_set.spawn(spawn_assembly_task(params, move |p| updater(p)));
                    }
                }
            } else if let Some(res) = join_set.try_join_next() {
                match res {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => {
                        log::error!("Assembly task failed: {err}");
                    }
                    Err(err) => {
                        log::error!("Assembly task join error: {err}");
                    }
                }
            } else {
                // No completed assembly tasks yet — yield briefly so we don't
                // spin-loop, but stay responsive to new channel entries and
                // cancellation.
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        return drain_join_set(&mut join_set).await;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
                }
            }
        }
    });

    handle
}

fn spawn_adaptive_adjuster(adaptive: &Arc<AdaptiveSemaphore>) -> CancellationToken {
    let cancel_token = CancellationToken::new();
    let adaptive = Arc::clone(adaptive);
    let token = cancel_token.clone();

    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(ADAPTIVE_WINDOW_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                _ = interval.tick() => {
                    adaptive.adjust();
                }
            }
        }
    });

    cancel_token
}

async fn check_needs_download(
    dest: &Path,
    chunk: &SophonManifestAssetChunk,
    game_dir: &Path,
    verify_cache: &Arc<DashMap<String, VerificationEntry>>,
) -> SophonResult<bool> {
    if tokio::fs::metadata(dest).await.is_err() {
        return Ok(true);
    }

    let chunk_size = chunk.chunk_size;
    let expected_md5 = chunk.chunk_compressed_hash_md5.clone();
    let cache = Arc::clone(verify_cache);
    let dest = dest.to_path_buf();
    let gd = game_dir.to_path_buf();

    let valid = tokio::task::spawn_blocking(move || {
        cache::check_file_md5_cached(&dest, chunk_size, &expected_md5, &gd, &cache).unwrap_or(false)
    })
    .await?;

    Ok(!valid)
}

async fn download_chunk_with_retries(
    chunk: &SophonManifestAssetChunk,
    client: &Client,
    chunk_download: &DownloadInfo,
    dest: &Path,
    ctx: &InstallContext,
    handle: &DownloadHandle,
) -> SophonResult<()> {
    let mut network_attempts: u32 = 0;
    let mut hash_failures: u32 = 0;

    loop {
        if handle.is_cancelled() {
            return Err(SophonError::Cancelled);
        }

        match super::download::download_chunk(client, chunk_download, chunk, dest, Some(handle))
            .await
        {
            Ok(()) => return Ok(()),
            Err(SophonError::Md5Mismatch { .. }) => {
                hash_failures += 1;
                if hash_failures >= MAX_HASH_RETRIES {
                    return Err(SophonError::DownloadFailed {
                        chunk: chunk.chunk_name.clone(),
                        attempts: hash_failures,
                        error: format!(
                            "hash verification failed after {} retries",
                            MAX_HASH_RETRIES
                        ),
                    });
                }
                log::warn!(
                    "MD5 mismatch for {} (hash retry {}/{}), re-downloading",
                    chunk.chunk_name,
                    hash_failures,
                    MAX_HASH_RETRIES
                );
                if let Err(err) = tokio::fs::remove_file(dest).await {
                    log::warn!(
                        "Failed to discard corrupted chunk {} before retry: {}",
                        chunk.chunk_name,
                        err
                    );
                }
                if cancelable_sleep(handle, retry_delay(hash_failures))
                    .await
                    .is_err()
                {
                    return Err(SophonError::Cancelled);
                }
            }
            Err(err) => {
                if !err.is_retryable() {
                    return Err(err);
                }
                network_attempts += 1;
                if network_attempts < MAX_RETRIES {
                    let err_msg = err.to_string();
                    (ctx.updater)(SophonProgress::Warning {
                        message: format!(
                            "Chunk {} failed (attempt {}/{}): {err_msg}",
                            chunk.chunk_name, network_attempts, MAX_RETRIES
                        ),
                    });
                    if cancelable_sleep(handle, retry_delay(network_attempts))
                        .await
                        .is_err()
                    {
                        return Err(SophonError::Cancelled);
                    }
                } else {
                    return Err(SophonError::DownloadFailed {
                        chunk: chunk.chunk_name.clone(),
                        attempts: MAX_RETRIES,
                        error: err.to_string(),
                    });
                }
            }
        }
    }
}

fn notify_assembly_ready(
    item_idx: usize,
    chunk_entries: &[Vec<FileEntry>],
    assemble_tx: &mpsc::Sender<(usize, usize)>,
) {
    let entries = match chunk_entries.get(item_idx) {
        Some(entries) => entries,
        None => return,
    };

    for (file_idx, tmp_dir_idx, pending) in entries {
        let prev = pending.fetch_sub(1, Ordering::AcqRel);
        if prev == 1 {
            let _ = assemble_tx.try_send((*file_idx, *tmp_dir_idx));
        }
    }
}

async fn process_download_item(
    item: DownloadItem,
    item_idx: usize,
    ctx: Arc<InstallContext>,
    chunk_entries: Arc<Vec<Vec<FileEntry>>>,
    assemble_tx: mpsc::Sender<(usize, usize)>,
    handle: DownloadHandle,
    adaptive: Arc<AdaptiveSemaphore>,
) -> SophonResult<()> {
    {
        let db = ctx.downloaded_bytes.load(Ordering::Relaxed);
        handle
            .wait_if_paused(
                &*ctx.updater,
                db + ctx.resume_bytes_offset.load(Ordering::Relaxed),
                ctx.total_bytes,
            )
            .await?;
    }

    let _permit = adaptive.acquire().await;

    let chunk = &ctx.all_files[item.file_idx].asset_chunks[item.chunk_idx];

    if !validate_chunk_name(&chunk.chunk_name) {
        return Err(SophonError::PathTraversal(chunk.chunk_name.clone().into()));
    }
    let dest = ctx.chunks_dir.join(assembly::chunk_filename(chunk));

    let mut was_actually_downloaded = false;
    let needs_download = if item.is_pre_downloaded {
        check_needs_download(&dest, chunk, &ctx.game_dir, &ctx.verify_cache).await?
    } else {
        true
    };
    if handle.is_cancelled() {
        return Err(SophonError::Cancelled);
    }
    if needs_download {
        download_chunk_with_retries(
            chunk,
            &item.client,
            &item.chunk_download,
            &dest,
            &ctx,
            &handle,
        )
        .await?;
        was_actually_downloaded = true;
    }

    if was_actually_downloaded && item.is_pre_downloaded {
        ctx.resume_bytes_offset
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_sub(chunk.chunk_size))
            })
            .ok();
    }

    ctx.downloaded_chunks
        .insert(chunk.chunk_name.clone(), chunk.chunk_size);

    let count = ctx.chunks_since_save.fetch_add(1, Ordering::Relaxed) + 1;
    if count.is_multiple_of(crate::commands::sophon_downloader::CHUNK_STATE_SAVE_INTERVAL) {
        let dc = Arc::clone(&ctx.downloaded_chunks);
        let saver = Arc::clone(&ctx.state_saver);
        let prev_handle = {
            let mut guard = ctx.last_save.lock().unwrap_or_else(|err| {
                log::error!("last_save mutex poisoned, recovering");
                err.into_inner()
            });
            guard.take()
        };
        if let Some(h) = prev_handle {
            let _ = h.await;
        }
        let new_handle = tokio::task::spawn_blocking(move || saver(&dc));
        {
            let mut guard = ctx.last_save.lock().unwrap_or_else(|err| {
                log::error!("last_save mutex poisoned, recovering");
                err.into_inner()
            });
            *guard = Some(new_handle);
        }
    }

    let db = if was_actually_downloaded {
        let old = ctx
            .downloaded_bytes
            .fetch_add(chunk.chunk_size, Ordering::Relaxed);
        old + chunk.chunk_size
    } else {
        ctx.downloaded_bytes.load(Ordering::Relaxed)
    };

    adaptive.record_bytes(chunk.chunk_size);

    let now = now_nanos();
    if now.saturating_sub(ctx.last_update.load(Ordering::Relaxed))
        >= PROGRESS_UPDATE_INTERVAL_MS * 1_000_000
    {
        let last_speed_nanos = ctx.last_speed_time.load(Ordering::Relaxed);
        let window_elapsed = if last_speed_nanos == 0 {
            0.0
        } else {
            now.saturating_sub(last_speed_nanos) as f64 / 1_000_000_000.0
        };
        let instant_window_speed = if window_elapsed >= 1.0 {
            let last_db = ctx.last_speed_bytes.load(Ordering::Relaxed);
            let window_bytes = db.saturating_sub(last_db);
            let window_speed = window_bytes as f64 / window_elapsed;
            ctx.last_speed_bytes.store(db, Ordering::Relaxed);
            ctx.last_speed_time.store(now, Ordering::Relaxed);
            window_speed
        } else {
            0.0
        };

        let speed_alpha =
            1.0 / (SPEED_SMOOTH_WINDOW_SECS * 1000.0 / PROGRESS_UPDATE_INTERVAL_MS as f64);
        let speed_bps =
            super::ewma_update(&ctx.smooth_speed_bps, instant_window_speed, speed_alpha);

        let eta_speed_bps = super::compute_eta_speed(&ctx.eta_speed_history, instant_window_speed);

        let remaining_bytes = ctx
            .total_bytes
            .saturating_sub(db + ctx.resume_bytes_offset.load(Ordering::Relaxed));
        let eta_seconds = if eta_speed_bps > 0.0 {
            remaining_bytes as f64 / eta_speed_bps
        } else {
            0.0
        };
        (ctx.updater)(SophonProgress::Downloading {
            downloaded_bytes: db + ctx.resume_bytes_offset.load(Ordering::Relaxed),
            total_bytes: ctx.total_bytes,
            speed_bps,
            eta_seconds,
        });
        ctx.last_update.store(now, Ordering::Relaxed);
    }

    notify_assembly_ready(item_idx, &chunk_entries, &assemble_tx);

    Ok(())
}

async fn run_downloads(
    ctx: Arc<InstallContext>,
    download_items: Vec<DownloadItem>,
    chunk_entries: Arc<Vec<Vec<FileEntry>>>,
    assemble_tx: &mpsc::Sender<(usize, usize)>,
    handle: DownloadHandle,
    adaptive: Arc<AdaptiveSemaphore>,
) -> Vec<SophonResult<()>> {
    let mut results = Vec::with_capacity(download_items.len());
    let mut join_set = tokio::task::JoinSet::new();
    const MAX_PENDING: usize = 32;

    for (item_idx, item) in download_items.into_iter().enumerate() {
        if handle.is_cancelled() {
            join_set.abort_all();
            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(r) => results.push(r),
                    Err(err) => results.push(Err(SophonError::JoinError(err))),
                }
            }
            results.push(Err(SophonError::Cancelled));
            break;
        }

        let ctx = Arc::clone(&ctx);
        let chunk_entries = Arc::clone(&chunk_entries);
        let assemble_tx = assemble_tx.clone();
        let handle = handle.clone();
        let adaptive = Arc::clone(&adaptive);

        join_set.spawn(async move {
            process_download_item(
                item,
                item_idx,
                ctx,
                chunk_entries,
                assemble_tx,
                handle,
                adaptive,
            )
            .await
        });

        if join_set.len() >= MAX_PENDING {
            match join_set.join_next().await {
                Some(Ok(r)) => results.push(r),
                Some(Err(err)) => results.push(Err(SophonError::JoinError(err))),
                None => break,
            }
        }
    }

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(r) => results.push(r),
            Err(err) => results.push(Err(SophonError::JoinError(err))),
        }
    }

    results
}

#[allow(clippy::too_many_arguments)]
async fn finalize_install(
    ctx: &InstallContext,
    results: Vec<SophonResult<()>>,
    deleted_files: Vec<String>,
    tag: &str,
    is_preinstall: bool,
    assembly_task: tokio::task::JoinHandle<SophonResult<()>>,
    assembly_cancel_token: CancellationToken,
    game_code: &str,
    vo_langs: &[String],
) -> SophonResult<()> {
    let cancelled = results
        .iter()
        .any(|r| matches!(r, Err(SophonError::Cancelled)));
    if cancelled {
        assembly_cancel_token.cancel(); // stop assembly before deleting chunks
        let _ = assembly_task.await; // drain before cleanup
        let cd = Arc::clone(&ctx.chunks_dir);
        #[allow(unused_must_use)]
        tokio::task::spawn_blocking(move || {
            if let Err(err) = fs::remove_dir_all(&*cd) {
                log::warn!(
                    "Failed to remove chunks directory {} on cancel: {}",
                    cd.display(),
                    err
                );
            }
        })
        .await;
        return Err(SophonError::Cancelled);
    }

    results.into_iter().find(|r| r.is_err()).transpose()?;
    assembly_task.await??;

    {
        let assembled = ctx.assembled_files.load(Ordering::Relaxed);
        let total = ctx.total_files;
        if assembled != total {
            log::warn!(
                "Sophon install completed but assembled_files ({}) != total_files ({}). {} files may be missing!",
                assembled,
                total,
                total - assembled,
            );
        } else {
            log::info!("Sophon install: all {total} files assembled successfully");
        }
    }

    {
        let dc = Arc::clone(&ctx.downloaded_chunks);
        let saver = Arc::clone(&ctx.state_saver);
        tokio::task::spawn_blocking(move || saver(&dc))
            .await
            .unwrap_or_else(|err| {
                log::error!("Final state save join error: {err}");
            });
    }

    {
        if let Err(err) = cache::save_verification_cache(&ctx.game_dir, &ctx.verify_cache) {
            log::warn!("Failed to save verification cache: {err}");
        }
    }

    if !deleted_files.is_empty() {
        let gd = ctx.game_dir.clone();
        tokio::task::spawn_blocking(move || {
            for rel in &deleted_files {
                if let Err(err) = validate_asset_name(rel) {
                    log::warn!("Skipping deleted file with invalid path: {err}");
                    continue;
                }
                let path = gd.join(rel);
                if let Err(err) = fs::remove_file(&path) {
                    let path_display = path.display();
                    log::warn!("Failed to delete file {path_display}: {err}");
                }
            }
        })
        .await?;
    }

    let gd = ctx.game_dir.clone();
    let tag_str = tag.to_owned();
    let is_pre = is_preinstall;
    tokio::task::spawn_blocking(move || {
        if is_pre {
            fs::write(gd.join(format!(".sophon_preinstall_{tag_str}")), &tag_str)
        } else {
            write_installed_tag(&gd, &tag_str)
        }
    })
    .await??;

    if game_code == "hkrpg" && !is_preinstall {
        let gd = ctx.game_dir.clone();
        let vl = vo_langs.to_vec();
        tokio::task::spawn_blocking(move || {
            if let Err(err) = super::game_filters::write_hkrpg_audio_lang_record(&gd, &vl) {
                log::warn!("Failed to write hkrpg audio language record: {err}");
            }
        })
        .await?;
        let gd = ctx.game_dir.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(err) = super::game_filters::write_hkrpg_app_info(&gd) {
                log::warn!("Failed to write hkrpg app.info: {err}");
            }
            if let Err(err) = super::game_filters::write_hkrpg_binary_version_files(&gd) {
                log::warn!("Failed to write hkrpg binary version files: {err}");
            }
        })
        .await?;
    } else if game_code == "hk4e" && !is_preinstall {
        let gd = ctx.game_dir.clone();
        let vl = vo_langs.to_vec();
        tokio::task::spawn_blocking(move || {
            if let Err(err) = super::game_filters::write_hk4e_audio_lang_record(&gd, &vl) {
                log::warn!("Failed to write hk4e audio language record: {err}");
            }
        })
        .await?;
        let gd = ctx.game_dir.clone();
        let vl = vo_langs.to_vec();
        let af = Arc::clone(&ctx.all_files);
        tokio::task::spawn_blocking(move || {
            if let Err(err) = super::game_filters::write_pkg_version_from_manifest(&gd, &af, &vl) {
                log::warn!("Failed to write hk4e pkg_version: {err}");
            }
        })
        .await?;
    } else if game_code == "nap" && !is_preinstall {
        let gd = ctx.game_dir.clone();
        let vl = vo_langs.to_vec();
        tokio::task::spawn_blocking(move || {
            if let Err(err) = super::game_filters::write_nap_audio_lang_records(&gd, &vl) {
                log::warn!("Failed to write nap audio language records: {err}");
            }
        })
        .await?;
    }

    Ok(())
}

pub struct InstallOptions {
    pub is_preinstall: bool,
    pub is_resume: bool,
    pub handle: DownloadHandle,
}

pub struct InstallCallbacks {
    pub updater: ProgressUpdater,
    pub state_saver: StateSaver,
}

fn chunk_still_valid_for_resume(chunk_name: &str, chunk_size: u64, chunks_dir: &Path) -> bool {
    if !validate_chunk_name(chunk_name) {
        return false;
    }
    let chunk_path = chunks_dir.join(format!("{chunk_name}.zstd"));
    match std::fs::metadata(&chunk_path) {
        Ok(meta) => meta.len() == chunk_size,
        Err(_) => false,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn install(
    installers: Vec<SophonInstaller>,
    game_dir: &Path,
    deleted_files: Vec<String>,
    tag: &str,
    resume: ResumeContext,
    options: InstallOptions,
    callbacks: InstallCallbacks,
    game_code: &str,
    vo_langs: &[String],
) -> SophonResult<()> {
    let chunks_dir = Arc::new(game_dir.join("chunks"));
    prepare_directories(game_dir, &chunks_dir).await?;

    // Create all new directories from asset manifests before they are
    // filtered out by `build_installer_data`. This ensures that new
    // directories introduced in updates actually exist on disk.
    for installer in &installers {
        for asset in &installer.manifest.assets {
            if asset.is_directory() {
                if let Err(err) = validate_asset_name(&asset.asset_name) {
                    log::warn!(
                        "Skipping directory with invalid asset_name \"{}\": {err}",
                        asset.asset_name
                    );
                    continue;
                }
                let dir_path = game_dir.join(&asset.asset_name);
                let dp = dir_path.clone();
                tokio::task::spawn_blocking(move || fs::create_dir_all(&dp))
                    .await?
                    .map_err(SophonError::from)?;
            }
        }
    }

    let ResumeContext {
        prev_manifest_hash,
        mut prev_downloaded_chunks,
    } = resume;
    let current_manifest_hash = combine_manifest_hashes(&installers);
    let manifest_changed = prev_manifest_hash != current_manifest_hash;
    if options.is_resume {
        // Validate that chunk files referenced in persisted state actually exist on
        // disk. Stale entries (e.g. user deleted game files between sessions)
        // would otherwise inflate the resume offset, causing incorrect progress
        // and skipped downloads.
        {
            let chunks_dir_validate = Arc::clone(&chunks_dir);
            prev_downloaded_chunks = tokio::task::spawn_blocking(move || {
                let before = prev_downloaded_chunks.len();
                prev_downloaded_chunks.retain(|chunk_name, chunk_size| {
                    chunk_still_valid_for_resume(chunk_name, *chunk_size, &chunks_dir_validate)
                });
                let removed = before - prev_downloaded_chunks.len();
                if removed > 0 {
                    log::warn!(
                        "Removed {}/{} stale chunk entries from resume state (chunks dir: {})",
                        removed,
                        before,
                        chunks_dir_validate.display()
                    );
                }
                prev_downloaded_chunks
            })
            .await?;
        }
        if manifest_changed {
            log::warn!(
                "Manifest changed on resume (old={}, new={}), re-verifying all chunks",
                prev_manifest_hash,
                current_manifest_hash
            );
            // The cached chunks reference names from the previous manifest.
            // Filter them down to names that still exist in the new manifest
            // so we don't waste I/O on MD5-validating chunks that have no
            // consumer.
            let manifest_chunk_names: HashSet<&str> = installers
                .iter()
                .flat_map(|inst| inst.manifest.assets.iter())
                .flat_map(|asset| asset.asset_chunks.iter())
                .map(|c| c.chunk_name.as_str())
                .collect();
            let before = prev_downloaded_chunks.len();
            prev_downloaded_chunks
                .retain(|chunk_name, _| manifest_chunk_names.contains(chunk_name.as_str()));
            let dropped = before - prev_downloaded_chunks.len();
            if dropped > 0 {
                log::warn!(
                    "Dropped {}/{} stale chunk entries whose names are absent from the new manifest",
                    dropped,
                    before
                );
            }
        } else {
            log::info!(
                "Manifest unchanged on resume (hash={}), preserving {} cached chunks",
                current_manifest_hash,
                prev_downloaded_chunks.len()
            );
        }
    }

    let (mut installer_data, mut all_files) = build_installer_data(installers);
    if game_code == "nap" {
        super::game_filters::filter_nap_installers(game_dir, &mut installer_data);
    }
    let mut file_installer_map: HashMap<String, usize> = HashMap::new();
    let mut idx: usize = 0;
    for (inst_idx, d) in installer_data.iter().enumerate() {
        for _ in 0..d.file_count {
            if idx < all_files.len() {
                file_installer_map.insert(all_files[idx].asset_name.clone(), inst_idx);
            }
            idx += 1;
        }
    }
    if game_code == "hkrpg" {
        super::game_filters::filter_hkrpg_asset_list(game_dir, &mut all_files);
    } else if game_code == "hk4e" {
        super::game_filters::filter_hk4e_asset_list(game_dir, &mut all_files, vo_langs);
    } else if game_code == "nap" {
        super::game_filters::filter_nap_asset_list(game_dir, &mut all_files);
    }
    let filtered_set: HashSet<String> = all_files
        .iter()
        .map(|f| f.asset_name.as_str().to_owned())
        .collect();
    all_files.retain(|f| filtered_set.contains(f.asset_name.as_str()));
    for d in &mut installer_data {
        d.file_count = 0;
    }
    for f in all_files.iter() {
        if let Some(inst_idx) = file_installer_map.get(&f.asset_name) {
            installer_data[*inst_idx].file_count += 1;
        }
    }
    let all_files: Arc<Vec<SophonManifestAssetProperty>> = Arc::new(all_files);
    let all_tmp_dirs: Arc<Vec<std::path::PathBuf>> = Arc::new(
        installer_data
            .iter()
            .map(|d| {
                let label = &d.label;
                game_dir.join(format!("tmp-{label}"))
            })
            .collect(),
    );

    let (total_compressed, total_files) = compute_totals(&all_files);
    log::info!(
        "Sophon install: {} total files across {} installers, {} compressed bytes",
        total_files,
        installer_data.len(),
        total_compressed,
    );
    for (i, d) in installer_data.iter().enumerate() {
        log::info!(
            "  installer[{}]: label={}, matching_field={}, files={}",
            i,
            d.label,
            d.matching_field,
            d.file_count,
        );
    }
    let verify_cache = Arc::new(cache::load_verification_cache(game_dir));

    let pre_downloaded: HashSet<String> = if options.is_resume {
        prev_downloaded_chunks.keys().cloned().collect()
    } else {
        HashSet::new()
    };

    let mut resume_bytes_offset: u64 = 0;
    let mut pre_assembled: u64 = 0;
    let mut completed_chunk_names: HashSet<String> = HashSet::new();
    let completed_indices = if options.is_resume {
        let total = all_files.len() as u64;
        (callbacks.updater)(SophonProgress::CalculatingDownloads {
            checked_files: 0,
            total_files: total,
        });

        let semaphore = Arc::new(tokio::sync::Semaphore::new(
            super::adaptive_max_concurrency(),
        ));
        let checked_files = Arc::new(AtomicU64::new(0));
        let resume_bytes_offset_arc = Arc::new(AtomicU64::new(0));
        let pre_assembled_arc = Arc::new(AtomicU64::new(0));
        let completed_chunk_names_arc: Arc<DashSet<String>> = Arc::new(DashSet::new());
        let indices_arc: Arc<DashSet<usize>> = Arc::new(DashSet::new());
        let files_to_delete: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));

        let calc_futures = all_files.iter().enumerate().map(|(file_idx, file)| {
            let permit = Arc::clone(&semaphore);
            let verify_cache = Arc::clone(&verify_cache);
            let game_dir = game_dir.to_path_buf();
            let checked_files = Arc::clone(&checked_files);
            let resume_bytes_offset_arc = Arc::clone(&resume_bytes_offset_arc);
            let pre_assembled_arc = Arc::clone(&pre_assembled_arc);
            let completed_chunk_names_arc = Arc::clone(&completed_chunk_names_arc);
            let indices_arc = Arc::clone(&indices_arc);
            let files_to_delete = Arc::clone(&files_to_delete);
            let updater = Arc::clone(&callbacks.updater);

            async move {
                let _permit = permit.acquire().await.ok()?;

                if file.asset_chunks.is_empty() {
                    indices_arc.insert(file_idx);
                    pre_assembled_arc.fetch_add(1, Ordering::Relaxed);
                } else {
                    if validate_asset_name(&file.asset_name).is_err() {
                        let checked = checked_files.fetch_add(1, Ordering::Relaxed) + 1;
                        if checked.is_multiple_of(500) {
                            updater(SophonProgress::CalculatingDownloads {
                                checked_files: checked,
                                total_files: total,
                            });
                        }
                        return None;
                    }

                    let target_path = game_dir.join(&file.asset_name);
                    let sz = file.asset_size;
                    let valid = if manifest_changed {
                        let tp = target_path.clone();
                        let md5 = file.asset_hash_md5.clone();
                        let ck = file.asset_name.clone();
                        let vc = Arc::clone(&verify_cache);
                        tokio::task::spawn_blocking(move || {
                            cache::check_file_md5_with_cache_key(&tp, sz, &md5, &ck, &vc)
                                .unwrap_or(false)
                        })
                        .await
                        .ok()?
                    } else {
                        tokio::fs::metadata(&target_path)
                            .await
                            .map(|m| m.len() == sz)
                            .unwrap_or(false)
                    };

                    if valid {
                        indices_arc.insert(file_idx);
                        let file_chunk_size: u64 = file
                            .asset_chunks
                            .iter()
                            .filter(|c| c.chunk_old_offset < 0)
                            .map(|c| c.chunk_size)
                            .fold(0u64, |acc, x| acc.saturating_add(x));
                        resume_bytes_offset_arc.fetch_add(file_chunk_size, Ordering::Relaxed);
                        for c in &file.asset_chunks {
                            completed_chunk_names_arc.insert(c.chunk_name.clone());
                        }
                        pre_assembled_arc.fetch_add(1, Ordering::Relaxed);
                    } else {
                        let needs_old_file =
                            file.asset_chunks.iter().any(|c| c.chunk_old_offset >= 0);
                        if !needs_old_file {
                            files_to_delete.lock().unwrap().push(target_path);
                        }
                    }
                }
                let checked = checked_files.fetch_add(1, Ordering::Relaxed) + 1;
                if checked.is_multiple_of(500) {
                    updater(SophonProgress::CalculatingDownloads {
                        checked_files: checked,
                        total_files: total,
                    });
                }
                Some(())
            }
        });

        futures_util::future::join_all(calc_futures).await;

        for path in files_to_delete.lock().unwrap().drain(..) {
            let _ = fs::remove_file(&path);
        }

        resume_bytes_offset = resume_bytes_offset_arc.load(Ordering::Relaxed);
        pre_assembled = pre_assembled_arc.load(Ordering::Relaxed);
        for name in completed_chunk_names_arc.iter() {
            completed_chunk_names.insert(name.key().clone());
        }

        (callbacks.updater)(SophonProgress::CalculatingDownloads {
            checked_files: total,
            total_files: total,
        });
        Some(indices_arc.iter().map(|r| *r.key()).collect())
    } else {
        None
    };

    for chunk_name in &pre_downloaded {
        if completed_chunk_names.contains(chunk_name.as_str()) {
            continue;
        }
        if let Some(&size) = prev_downloaded_chunks.get(chunk_name) {
            resume_bytes_offset += size;
        }
    }

    let initial_chunks = if options.is_resume {
        prev_downloaded_chunks
    } else {
        HashMap::new()
    };

    let adaptive_assembly = Arc::new(AdaptiveAssembly::new());
    let initial_dashmap: DashMap<String, u64> = DashMap::from_iter(initial_chunks);
    let ctx = Arc::new(InstallContext {
        chunks_dir: Arc::clone(&chunks_dir),
        game_dir: game_dir.to_path_buf(),
        all_tmp_dirs: Arc::clone(&all_tmp_dirs),
        all_files: Arc::clone(&all_files),
        downloaded_bytes: Arc::new(AtomicU64::new(0)),
        assembled_files: Arc::new(AtomicU64::new(pre_assembled)),
        total_bytes: total_compressed,
        total_files,
        resume_bytes_offset: Arc::new(AtomicU64::new(resume_bytes_offset)),
        verify_cache,
        chunk_refcounts: Arc::new(DashMap::new()),
        last_assembly_update: Arc::new(Mutex::new(Instant::now())),
        last_update: Arc::new(AtomicU64::new(now_nanos())),
        smooth_speed_bps: Arc::new(AtomicU64::new(0)),
        eta_speed_history: Arc::new(Mutex::new(VecDeque::new())),
        last_speed_bytes: Arc::new(AtomicU64::new(0)),
        last_speed_time: Arc::new(AtomicU64::new(now_nanos())),
        updater: Arc::clone(&callbacks.updater),
        downloaded_chunks: Arc::new(initial_dashmap),
        chunks_since_save: Arc::new(AtomicU64::new(0)),
        last_save: Arc::new(Mutex::new(None)),
        state_saver: callbacks.state_saver,
        adaptive_assembly: Arc::clone(&adaptive_assembly),
    });

    let (assemble_tx, assemble_rx) = mpsc::channel::<(usize, usize)>(ASSEMBLY_CHANNEL_SIZE);
    // Single token shared by AdaptiveAssembly::spawn_adjuster and the coordinator
    // loop. Cancelling it cuts the RAM adjuster AND wakes the recv() race
    // above.
    let assembly_cancel_token = tokio_util::sync::CancellationToken::new();
    let assembly_task =
        spawn_assembly_coordinator(&ctx, assemble_rx, assembly_cancel_token.clone());

    let (download_items, chunk_entries) = build_download_state(
        installer_data,
        &ctx,
        &assemble_tx,
        completed_indices.as_ref(),
        &pre_downloaded,
    )
    .await?;

    {
        let initial_offset = ctx.resume_bytes_offset.load(Ordering::Relaxed);
        (ctx.updater)(SophonProgress::Downloading {
            downloaded_bytes: initial_offset,
            total_bytes: ctx.total_bytes,
            speed_bps: 0.0,
            eta_seconds: 0.0,
        });
        ctx.last_update.store(now_nanos(), Ordering::Relaxed);
    }

    let adaptive = Arc::new(AdaptiveSemaphore::new());
    let cancel_token = spawn_adaptive_adjuster(&adaptive);

    let results = run_downloads(
        Arc::clone(&ctx),
        download_items,
        chunk_entries,
        &assemble_tx,
        options.handle,
        Arc::clone(&adaptive),
    )
    .await;

    cancel_token.cancel();

    {
        let handle = {
            let mut guard = ctx.last_save.lock().unwrap_or_else(|err| {
                log::error!("last_save mutex poisoned, recovering");
                err.into_inner()
            });
            guard.take()
        };
        if let Some(h) = handle {
            let _ = h.await;
        }
    }

    drop(assemble_tx);
    let result = finalize_install(
        &ctx,
        results,
        deleted_files,
        tag,
        options.is_preinstall,
        assembly_task,
        assembly_cancel_token.clone(),
        game_code,
        vo_langs,
    )
    .await;
    assembly_cancel_token.cancel();
    result
}

pub async fn verify_integrity(
    client: &Client,
    game_id: &str,
    vo_lang: &str,
    game_dir: &Path,
    mut emit: impl FnMut(SophonProgress) + Send + 'static,
) -> SophonResult<()> {
    emit(SophonProgress::Verifying {
        scanned_files: 0,
        total_files: 0,
        error_count: 0,
    });

    let tag = read_installed_tag(game_dir).ok_or(SophonError::NoInstalledVersion)?;

    let (branch, _) = api::fetch_front_door(client, game_id).await?;
    let build = api::fetch_build(
        client,
        branch.main.as_ref().ok_or(SophonError::NoGameManifest)?,
        Some(&tag),
    )
    .await?;

    let qualifying: Vec<&SophonManifestMeta> = build
        .manifests
        .iter()
        .filter(|m| {
            m.matching_field == "game"
                || api::vo_lang_matches(&m.matching_field, vo_lang)
                || !api::is_known_vo_locale(&m.matching_field)
        })
        .collect();

    if qualifying.is_empty() {
        return Err(SophonError::NoGameManifest);
    }

    let mut manifest_results: Vec<SophonManifestProto> = Vec::with_capacity(qualifying.len());
    let mut chunk_downloads: Vec<&DownloadInfo> = Vec::with_capacity(qualifying.len());
    for meta in &qualifying {
        let result =
            api::fetch_manifest(client, &meta.manifest_download, &meta.manifest.id).await?;
        manifest_results.push(result.manifest);
        chunk_downloads.push(&meta.chunk_download);
    }

    let all_assets: Vec<(&SophonManifestAssetProperty, &DownloadInfo)> = manifest_results
        .iter()
        .zip(chunk_downloads.iter())
        .flat_map(|(manifest, dl)| {
            manifest
                .assets
                .iter()
                .filter(|a| !a.is_directory())
                .map(|a| (a, *dl))
        })
        .collect();

    let total_files = all_assets.len() as u64;
    let verify_cache = Arc::new(cache::load_verification_cache(game_dir));
    let chunks_dir = game_dir.join("chunks");
    let mut last_emit = Instant::now();

    // Phase 1: Verify all files in parallel (bounded by semaphore)
    let semaphore = Arc::new(tokio::sync::Semaphore::new(
        super::adaptive_max_concurrency(),
    ));
    let scanned_count = Arc::new(AtomicU64::new(0));
    let error_count = Arc::new(AtomicU64::new(0));

    let verify_futures = all_assets.into_iter().map(|(asset, chunk_download)| {
        let permit = Arc::clone(&semaphore);
        let verify_cache = Arc::clone(&verify_cache);
        let game_dir = game_dir.to_path_buf();
        let scanned_count = Arc::clone(&scanned_count);
        let error_count = Arc::clone(&error_count);

        async move {
            let _permit = permit.acquire().await.ok()?;

            if let Err(err) = validate_asset_name(&asset.asset_name) {
                log::warn!("Skipping file with invalid asset_name during verification: {err}");
                return None;
            }

            let file_path = game_dir.join(&asset.asset_name);
            let asset_size = asset.asset_size;
            let asset_md5 = asset.asset_hash_md5.clone();

            let is_valid = tokio::task::spawn_blocking({
                let file_path = file_path.clone();
                move || {
                    cache::check_file_md5_cached(
                        &file_path,
                        asset_size,
                        &asset_md5,
                        &game_dir,
                        &verify_cache,
                    )
                    .unwrap_or(false)
                }
            })
            .await
            .ok()?;

            let scanned = scanned_count.fetch_add(1, Ordering::Relaxed) + 1;

            if !is_valid {
                error_count.fetch_add(1, Ordering::Relaxed);
                Some((asset, chunk_download, file_path))
            } else {
                if scanned.is_multiple_of(100) {
                    log::debug!("Verified {}/{} files", scanned, total_files);
                }
                None
            }
        }
    });

    // Collect failed verifications for re-download
    let failed_verifications: Vec<_> = futures_util::future::join_all(verify_futures)
        .await
        .into_iter()
        .flatten()
        .collect();

    // Phase 2: Re-download failed files sequentially (to avoid overwhelming the
    // network)
    for (asset, chunk_download, file_path) in failed_verifications {
        emit(SophonProgress::Warning {
            message: format!(
                "File {} failed integrity check, re-downloading",
                asset.asset_name
            ),
        });

        if let Err(err) = redownload_asset(
            client,
            asset,
            chunk_download,
            &chunks_dir,
            game_dir,
            &file_path,
            &mut emit,
            &verify_cache,
        )
        .await
        {
            let asset_name = &asset.asset_name;
            emit(SophonProgress::Error {
                message: format!("Failed to re-download {asset_name}: {err}"),
            });
        }

        if last_emit.elapsed() >= Duration::from_millis(PROGRESS_UPDATE_INTERVAL_MS) {
            emit(SophonProgress::Verifying {
                scanned_files: scanned_count.load(Ordering::Relaxed),
                total_files,
                error_count: error_count.load(Ordering::Relaxed),
            });
            last_emit = Instant::now();
        }
    }

    emit(SophonProgress::Verifying {
        scanned_files: total_files,
        total_files,
        error_count: error_count.load(Ordering::Relaxed),
    });

    emit(SophonProgress::Finished);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn redownload_asset(
    client: &Client,
    asset: &SophonManifestAssetProperty,
    chunk_download: &DownloadInfo,
    chunks_dir: &Path,
    game_dir: &Path,
    file_path: &Path,
    emit: &mut (impl FnMut(SophonProgress) + Send + 'static),
    verify_cache: &DashMap<String, VerificationEntry>,
) -> SophonResult<()> {
    fs::create_dir_all(chunks_dir)?;

    for chunk in &asset.asset_chunks {
        if !validate_chunk_name(&chunk.chunk_name) {
            return Err(SophonError::PathTraversal(chunk.chunk_name.clone().into()));
        }
        let chunk_path = chunks_dir.join(assembly::chunk_filename(chunk));
        let needs_download = !chunk_path.exists()
            || !cache::check_file_md5_cached(
                &chunk_path,
                chunk.chunk_size,
                &chunk.chunk_compressed_hash_md5,
                game_dir,
                verify_cache,
            )
            .unwrap_or(false);

        if needs_download {
            let chunk_name = &chunk.chunk_name;
            emit(SophonProgress::Warning {
                message: format!("Re-downloading chunk {chunk_name}"),
            });
            download::download_chunk(client, chunk_download, chunk, &chunk_path, None).await?;
        }
    }

    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let _ = fs::remove_file(file_path);

    let tmp_dir_name = format!(
        "tmp-verify-{}",
        asset.asset_name.replace(['/', '\\', ':'], "_")
    );
    let tmp_dir = game_dir.join(&tmp_dir_name);
    fs::create_dir_all(&tmp_dir)?;
    let result = assembly::assemble_file(
        asset,
        game_dir,
        chunks_dir,
        &tmp_dir,
        &DashMap::new(),
        verify_cache,
    );
    let _ = fs::remove_dir_all(&tmp_dir);
    result?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::sophon_downloader::api_scrape::Compression;

    fn make_chunk(name: &str, size: u64) -> SophonManifestAssetChunk {
        SophonManifestAssetChunk {
            chunk_name: name.into(),
            chunk_decompressed_hash_md5: String::new(),
            chunk_on_file_offset: 0,
            chunk_size: size,
            chunk_size_decompressed: size,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: String::new(),
            chunk_old_offset: -1,
        }
    }

    fn make_file(
        name: &str,
        md5: &str,
        chunks: Vec<SophonManifestAssetChunk>,
    ) -> SophonManifestAssetProperty {
        let size: u64 = chunks.iter().map(|c| c.chunk_size_decompressed).sum();
        SophonManifestAssetProperty {
            asset_name: name.into(),
            asset_chunks: chunks,
            asset_type: 0,
            asset_size: size,
            asset_hash_md5: md5.into(),
        }
    }

    fn make_dir(name: &str) -> SophonManifestAssetProperty {
        SophonManifestAssetProperty {
            asset_name: name.into(),
            asset_chunks: vec![],
            asset_type: 64,
            asset_size: 0,
            asset_hash_md5: String::new(),
        }
    }

    fn make_download_info() -> DownloadInfo {
        DownloadInfo {
            encryption: 0,
            password: String::new(),
            compression: Compression::None,
            url_prefix: "https://example.com".into(),
            url_suffix: "chunks".into(),
        }
    }

    fn make_installer_data(files: Vec<SophonManifestAssetProperty>) -> InstallerData {
        InstallerData {
            client: Arc::new(Client::new()),
            chunk_download: Arc::new(make_download_info()),
            file_count: files.len(),
            label: "test".into(),
            matching_field: "game".into(),
        }
    }

    fn make_sophon_installer(hash: &str) -> SophonInstaller {
        SophonInstaller {
            client: Client::new(),
            manifest: SophonManifestProto { assets: vec![] },
            chunk_download: make_download_info(),
            label: "test".into(),
            matching_field: "game".into(),
            manifest_hash: hash.into(),
        }
    }

    #[test]
    fn compute_totals_no_dupes() {
        let files = vec![
            make_file(
                "a.pak",
                "aa",
                vec![make_chunk("c1", 100), make_chunk("c2", 200)],
            ),
            make_file("b.pak", "bb", vec![make_chunk("c3", 300)]),
        ];
        let (bytes, files_count) = compute_totals(&files);
        assert_eq!(bytes, 600);
        assert_eq!(files_count, 2);
    }

    #[test]
    fn compute_totals_with_dedup() {
        let files = vec![
            make_file("a.pak", "aa", vec![make_chunk("shared", 500)]),
            make_file("b.pak", "bb", vec![make_chunk("shared", 500)]),
        ];
        let (bytes, files_count) = compute_totals(&files);
        assert_eq!(bytes, 500);
        assert_eq!(files_count, 2);
    }

    #[test]
    fn compute_totals_empty() {
        let files: Vec<SophonManifestAssetProperty> = vec![];
        let (bytes, files_count) = compute_totals(&files);
        assert_eq!(bytes, 0);
        assert_eq!(files_count, 0);
    }

    #[test]
    fn compute_totals_same_name_different_size() {
        let files = vec![
            make_file("a.pak", "aa", vec![make_chunk("shared", 500)]),
            make_file("b.pak", "bb", vec![make_chunk("shared", 600)]),
        ];
        let (bytes, files_count) = compute_totals(&files);
        assert_eq!(bytes, 500);
        assert_eq!(files_count, 2);
    }

    #[test]
    fn compute_diff_files_all_new() {
        let new_manifest = SophonManifestProto {
            assets: vec![
                make_file("a.pak", "aa", vec![]),
                make_file("b.pak", "bb", vec![]),
                make_file("c.pak", "cc", vec![]),
            ],
        };
        let old_md5_map = HashMap::new();
        let diff = compute_diff_files(new_manifest, &old_md5_map);
        assert_eq!(diff.len(), 3);
    }

    #[test]
    fn compute_diff_files_unchanged_excluded() {
        let new_manifest = SophonManifestProto {
            assets: vec![make_file("a.pak", "aa", vec![])],
        };
        let mut old_md5_map = HashMap::new();
        old_md5_map.insert("a.pak".to_string(), "aa".to_string());
        let diff = compute_diff_files(new_manifest, &old_md5_map);
        assert!(diff.is_empty());
    }

    #[test]
    fn compute_diff_files_changed_included() {
        let new_manifest = SophonManifestProto {
            assets: vec![make_file("a.pak", "new_md5", vec![])],
        };
        let mut old_md5_map = HashMap::new();
        old_md5_map.insert("a.pak".to_string(), "old_md5".to_string());
        let diff = compute_diff_files(new_manifest, &old_md5_map);
        assert_eq!(diff.len(), 1);
    }

    #[test]
    fn compute_diff_files_dirs_included() {
        let new_manifest = SophonManifestProto {
            assets: vec![make_dir("GameData"), make_file("a.pak", "aa", vec![])],
        };
        let diff = compute_diff_files(new_manifest, &HashMap::new());
        assert_eq!(diff.len(), 2);
        let names: Vec<&str> = diff.iter().map(|f| f.asset_name.as_str()).collect();
        assert!(names.contains(&"GameData"));
        assert!(names.contains(&"a.pak"));
    }

    #[test]
    fn compute_diff_files_mixed() {
        let new_manifest = SophonManifestProto {
            assets: vec![
                make_file("new.pak", "nn", vec![]),
                make_file("changed.pak", "new_md5", vec![]),
                make_file("unchanged.pak", "same", vec![]),
                make_dir("somedir"),
            ],
        };
        let mut old_md5_map = HashMap::new();
        old_md5_map.insert("changed.pak".to_string(), "old_md5".to_string());
        old_md5_map.insert("unchanged.pak".to_string(), "same".to_string());
        let diff = compute_diff_files(new_manifest, &old_md5_map);
        assert_eq!(diff.len(), 3);
        let names: Vec<&str> = diff.iter().map(|f| f.asset_name.as_str()).collect();
        assert!(names.contains(&"new.pak"));
        assert!(names.contains(&"changed.pak"));
        assert!(names.contains(&"somedir")); // directories are included in diff
    }

    #[test]
    fn collect_deleted_files_basic() {
        let old = SophonManifestProto {
            assets: vec![
                make_file("A", "a1", vec![]),
                make_file("B", "b1", vec![]),
                make_file("C", "c1", vec![]),
            ],
        };
        let mut new_names = HashSet::new();
        new_names.insert("A");
        new_names.insert("D");
        let deleted = collect_deleted_files(&old, &new_names);
        assert_eq!(deleted.len(), 2);
        assert!(deleted.contains(&"B".to_string()));
        assert!(deleted.contains(&"C".to_string()));
    }

    #[test]
    fn collect_deleted_files_none() {
        let old = SophonManifestProto {
            assets: vec![make_file("A", "a1", vec![]), make_file("B", "b1", vec![])],
        };
        let mut new_names = HashSet::new();
        new_names.insert("A");
        new_names.insert("B");
        let deleted = collect_deleted_files(&old, &new_names);
        assert!(deleted.is_empty());
    }

    #[test]
    fn collect_deleted_files_dirs_excluded() {
        let old = SophonManifestProto {
            assets: vec![make_dir("old_dir"), make_file("A", "a1", vec![])],
        };
        let new_names: HashSet<&str> = HashSet::from(["A"]);
        let deleted = collect_deleted_files(&old, &new_names);
        assert!(deleted.is_empty());
    }

    #[test]
    fn build_old_md5_map_basic() {
        let manifest = SophonManifestProto {
            assets: vec![
                make_file("a.pak", "md5_a", vec![]),
                make_file("b.pak", "md5_b", vec![]),
                make_dir("dir"),
            ],
        };
        let map = build_old_md5_map(manifest);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("a.pak"), Some(&"md5_a".to_string()));
        assert_eq!(map.get("b.pak"), Some(&"md5_b".to_string()));
    }

    #[test]
    fn combine_manifest_hashes_deterministic() {
        let installers = vec![
            make_sophon_installer("hash_a"),
            make_sophon_installer("hash_b"),
        ];
        let h1 = combine_manifest_hashes(&installers);
        let h2 = combine_manifest_hashes(&installers);
        assert_eq!(h1, h2);
    }

    #[test]
    fn combine_manifest_hashes_order_independent() {
        let installers_ab = vec![
            make_sophon_installer("hash_a"),
            make_sophon_installer("hash_b"),
        ];
        let installers_ba = vec![
            make_sophon_installer("hash_b"),
            make_sophon_installer("hash_a"),
        ];
        assert_eq!(
            combine_manifest_hashes(&installers_ab),
            combine_manifest_hashes(&installers_ba),
        );
    }

    #[tokio::test]
    async fn notify_assembly_ready_single_file_ready() {
        let (tx, mut rx) = mpsc::channel::<(usize, usize)>(16);

        let pending: PendingCount = Arc::new(AtomicUsize::new(1usize));
        let chunk_entries: Vec<Vec<FileEntry>> = vec![vec![(0usize, 0usize, Arc::clone(&pending))]];

        notify_assembly_ready(0, &chunk_entries, &tx);

        let received = rx.try_recv();
        assert!(received.is_ok(), "file should be sent to assembly channel");
        assert_eq!(received.unwrap(), (0, 0));
        assert_eq!(pending.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn notify_assembly_ready_chunk_not_in_map() {
        let chunk_entries: Vec<Vec<FileEntry>> = vec![];
        let (tx, rx) = mpsc::channel::<(usize, usize)>(16);
        drop(rx);

        notify_assembly_ready(999, &chunk_entries, &tx);
    }

    #[tokio::test]
    async fn check_needs_download_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("does_not_exist.bin");
        let chunk = make_chunk("c1", 100);
        let cache = Arc::new(DashMap::new());

        let needs = check_needs_download(&dest, &chunk, dir.path(), &cache)
            .await
            .unwrap();
        assert!(needs);
    }

    #[tokio::test]
    async fn check_needs_download_valid_cached_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("cached.bin");
        let data = b"hello cache";
        std::fs::write(&file_path, data).unwrap();

        let md5_hex = {
            let mut hasher = md5::Md5::new();
            hasher.update(data);
            hex::encode(hasher.finalize())
        };

        let metadata = std::fs::metadata(&file_path).unwrap();
        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let cache: Arc<DashMap<String, VerificationEntry>> = Arc::new(DashMap::new());
        let rel_path = file_path
            .strip_prefix(dir.path())
            .unwrap()
            .to_string_lossy()
            .to_string();
        cache.insert(
            rel_path,
            VerificationEntry {
                size: data.len() as u64,
                md5: md5_hex.clone(),
                mtime_secs: mtime,
            },
        );

        let mut chunk = make_chunk("c1", data.len() as u64);
        chunk.chunk_compressed_hash_md5 = md5_hex;

        let needs = check_needs_download(&file_path, &chunk, dir.path(), &cache)
            .await
            .unwrap();
        assert!(!needs);
    }

    #[tokio::test]
    async fn download_chunk_with_retries_success_on_first() {
        use crate::commands::sophon_downloader::api_scrape::Compression;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let data = b"chunk payload".to_vec();
        let expected_md5 = hex::encode(md5::Md5::digest(&data));

        let data_len = data.len() as u64;
        let chunk = SophonManifestAssetChunk {
            chunk_name: "test_retry_chunk".to_string(),
            chunk_decompressed_hash_md5: String::new(),
            chunk_on_file_offset: 0,
            chunk_size: data_len,
            chunk_size_decompressed: data_len,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: expected_md5,
            chunk_old_offset: -1,
        };

        let dl_info = Arc::new(DownloadInfo {
            encryption: 0,
            password: String::new(),
            compression: Compression::None,
            url_prefix: format!("{}/", server.uri()),
            url_suffix: "chunks".to_string(),
        });

        Mock::given(method("GET"))
            .and(path("chunks/test_retry_chunk"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Arc::new(Client::new());
        let chunk_download = dl_info;

        Mock::given(method("GET"))
            .and(path("chunks/test_retry_chunk"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("test_retry_chunk.zstd");

        let ctx = Arc::new(InstallContext {
            chunks_dir: Arc::new(dir.path().to_path_buf()),
            game_dir: dir.path().to_path_buf(),
            all_tmp_dirs: Arc::new(vec![]),
            all_files: Arc::new(vec![]),
            downloaded_bytes: Arc::new(AtomicU64::new(0)),
            assembled_files: Arc::new(AtomicU64::new(0)),
            total_bytes: 0,
            total_files: 0,
            resume_bytes_offset: Arc::new(AtomicU64::new(0)),
            verify_cache: Arc::new(DashMap::new()),
            chunk_refcounts: Arc::new(DashMap::new()),
            last_assembly_update: Arc::new(Mutex::new(Instant::now())),
            last_update: Arc::new(AtomicU64::new(now_nanos())),
            smooth_speed_bps: Arc::new(AtomicU64::new(0)),
            eta_speed_history: Arc::new(Mutex::new(VecDeque::new())),
            last_speed_bytes: Arc::new(AtomicU64::new(0)),
            last_speed_time: Arc::new(AtomicU64::new(now_nanos())),
            updater: Arc::new(|_| {}),
            downloaded_chunks: Arc::new(DashMap::new()),
            chunks_since_save: Arc::new(AtomicU64::new(0)),
            last_save: Arc::new(Mutex::new(None)),
            state_saver: Arc::new(|_| {}),
            adaptive_assembly: Arc::new(AdaptiveAssembly::new()),
        });

        let handle = DownloadHandle::new();

        let result =
            download_chunk_with_retries(&chunk, &client, &chunk_download, &dest, &ctx, &handle)
                .await;
        assert!(result.is_ok());
    }

    /// When a chunk download recovers via HTTP Range resume and still fails
    /// MD5 verification, the partial file must be discarded before retrying.
    /// Otherwise the next attempt would append fresh bytes on top of
    /// corrupted ones and the MD5 would still never match.
    #[tokio::test]
    async fn download_chunk_with_retries_mismatch_discards_partial() {
        use crate::commands::sophon_downloader::api_scrape::Compression;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let data = b"chunk payload that has the wrong hash".to_vec();
        // Intentionally wrong hash so the chunk always fails MD5 verification.
        let _wrong_md5 = "00000000000000000000000000000000";

        let wrong_md5 = hex::encode(md5::Md5::digest(b"wrong_data"));
        let data_len = data.len() as u64;
        let chunk = SophonManifestAssetChunk {
            chunk_name: "discard_partial_chunk".to_string(),
            chunk_decompressed_hash_md5: String::new(),
            chunk_on_file_offset: 0,
            chunk_size: data_len,
            chunk_size_decompressed: data_len,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: wrong_md5.to_string(),
            chunk_old_offset: -1,
        };

        let server_uri = server.uri();
        let dl_info = Arc::new(DownloadInfo {
            encryption: 0,
            password: String::new(),
            compression: Compression::None,
            url_prefix: format!("{server_uri}/"),
            url_suffix: "chunks".to_string(),
        });

        Mock::given(method("GET"))
            .and(path("chunks/discard_partial_chunk"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-length", data.len().to_string()),
            )
            .mount(&server)
            .await;

        let client = Arc::new(Client::new());
        let chunk_download = dl_info;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("discard_partial_chunk.zstd");
        // Pre-create the file so we can check whether it gets deleted.
        tokio::fs::write(&dest, b"corrupted-existing-content")
            .await
            .unwrap();

        let ctx = Arc::new(InstallContext {
            chunks_dir: Arc::new(dir.path().to_path_buf()),
            game_dir: dir.path().to_path_buf(),
            all_tmp_dirs: Arc::new(vec![]),
            all_files: Arc::new(vec![]),
            downloaded_bytes: Arc::new(AtomicU64::new(0)),
            assembled_files: Arc::new(AtomicU64::new(0)),
            total_bytes: 0,
            total_files: 0,
            resume_bytes_offset: Arc::new(AtomicU64::new(0)),
            verify_cache: Arc::new(DashMap::new()),
            chunk_refcounts: Arc::new(DashMap::new()),
            last_assembly_update: Arc::new(Mutex::new(Instant::now())),
            last_update: Arc::new(AtomicU64::new(now_nanos())),
            smooth_speed_bps: Arc::new(AtomicU64::new(0)),
            eta_speed_history: Arc::new(Mutex::new(VecDeque::new())),
            last_speed_bytes: Arc::new(AtomicU64::new(0)),
            last_speed_time: Arc::new(AtomicU64::new(now_nanos())),
            updater: Arc::new(|_| {}),
            downloaded_chunks: Arc::new(DashMap::new()),
            chunks_since_save: Arc::new(AtomicU64::new(0)),
            last_save: Arc::new(Mutex::new(None)),
            state_saver: Arc::new(|_| {}),
            adaptive_assembly: Arc::new(AdaptiveAssembly::new()),
        });

        let handle = DownloadHandle::new();

        let result =
            download_chunk_with_retries(&chunk, &client, &chunk_download, &dest, &ctx, &handle)
                .await;
        // After MAX_HASH_RETRIES attempts, the operation should fail; the
        // point of this test is that the partial file was discarded on each
        // Md5Mismatch retry (so the retry attempt always starts from size 0).
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("hash verification failed"),
            "expected hash failure, got: {err_msg}"
        );
        // The destination file must NOT exist after the failures because we
        // discarded it on every mismatch.
        assert!(
            !dest.exists(),
            "partial file should be deleted after MD5 mismatch retries"
        );
    }

    #[test]
    fn compute_totals_filters_old_reuse_chunks() {
        let chunk_new = SophonManifestAssetChunk {
            chunk_name: "new_chunk".into(),
            chunk_size: 500,
            chunk_size_decompressed: 500,
            chunk_old_offset: -1,
            chunk_decompressed_hash_md5: String::new(),
            chunk_on_file_offset: 0,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: String::new(),
        };
        let chunk_reuse = SophonManifestAssetChunk {
            chunk_name: "reuse_chunk".into(),
            chunk_size: 300,
            chunk_size_decompressed: 300,
            chunk_old_offset: 42,
            chunk_decompressed_hash_md5: String::new(),
            chunk_on_file_offset: 0,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: String::new(),
        };
        let file1 = SophonManifestAssetProperty {
            asset_name: "a.pak".into(),
            asset_chunks: vec![chunk_new, chunk_reuse],
            asset_type: 0,
            asset_hash_md5: String::new(),
            asset_size: 0,
        };
        let (bytes, files_count) = compute_totals(&[file1]);
        assert_eq!(bytes, 500, "old-reuse chunk should be excluded");
        assert_eq!(files_count, 1);
    }

    #[test]
    fn compute_totals_filters_directories() {
        let file1 = make_file("a.pak", "aa", vec![make_chunk("c1", 100)]);
        let dir1 = make_dir("GameData");
        let (bytes, files_count) = compute_totals(&[dir1, file1]);
        assert_eq!(bytes, 100);
        assert_eq!(files_count, 2);
    }

    #[test]
    fn build_installer_data_filters_directories() {
        let installer = SophonInstaller {
            client: Client::new(),
            manifest: SophonManifestProto {
                assets: vec![
                    make_dir("GameData"),
                    make_file("a.pak", "aa", vec![make_chunk("c1", 100)]),
                ],
            },
            chunk_download: make_download_info(),
            label: "test".into(),
            matching_field: "game".into(),
            manifest_hash: "abc".into(),
        };

        let (data, all_files) = build_installer_data(vec![installer]);
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].file_count, 1);
        assert_eq!(all_files.len(), 1);
        assert_eq!(all_files[0].asset_name, "a.pak");
    }

    #[test]
    fn combine_manifest_hashes_empty() {
        let installers: Vec<SophonInstaller> = vec![];
        let hash = combine_manifest_hashes(&installers);
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 16);
    }

    #[test]
    fn compute_diff_files_all_unchanged() {
        let new_manifest = SophonManifestProto {
            assets: vec![
                make_file("a.pak", "aa", vec![]),
                make_file("b.pak", "bb", vec![]),
            ],
        };
        let mut old_md5_map = HashMap::new();
        old_md5_map.insert("a.pak".to_string(), "aa".to_string());
        old_md5_map.insert("b.pak".to_string(), "bb".to_string());
        let diff = compute_diff_files(new_manifest, &old_md5_map);
        assert!(diff.is_empty());
    }

    #[test]
    fn chunk_still_valid_for_resume_missing() {
        let tmp = std::env::temp_dir().join(format!(
            "elysiae_resume_missing_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(!chunk_still_valid_for_resume("missing_chunk", 123, &tmp));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn chunk_still_valid_for_resume_size_mismatch() {
        let tmp = std::env::temp_dir().join(format!(
            "elysiae_resume_size_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let chunk_path = tmp.join("OK_chunk.zstd");
        std::fs::write(&chunk_path, b"abc").unwrap();
        assert!(!chunk_still_valid_for_resume("OK_chunk", 5, &tmp));
        assert!(!chunk_still_valid_for_resume("OK_chunk", 100, &tmp));
        assert!(chunk_still_valid_for_resume("OK_chunk", 3, &tmp));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn chunk_still_valid_for_resume_invalid_name() {
        let tmp = std::env::temp_dir().join(format!(
            "elysiae_resume_invalid_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("ok.zstd"), b"hi").unwrap();
        assert!(!chunk_still_valid_for_resume("../escape", 2, &tmp));
        assert!(!chunk_still_valid_for_resume("", 0, &tmp));
        assert!(!chunk_still_valid_for_resume("name/with/slash", 2, &tmp));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
