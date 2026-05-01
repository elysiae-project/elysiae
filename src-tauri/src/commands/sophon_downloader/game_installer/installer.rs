use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use futures_util::StreamExt;
use reqwest::Client;
use sha2::{Digest, Sha256};
use tauri_plugin_log::log;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::adaptive_assembly::AdaptiveAssembly;
use super::adaptive_download::AdaptiveSemaphore;
use super::api::{fetch_build, fetch_front_door, is_known_vo_locale, vo_lang_matches};
use super::assembly::{self, AssemblyTaskParams, cleanup_tmp_files, spawn_assembly_task};
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
    last_update: Arc<Mutex<Instant>>,
    download_start: Instant,
    updater: ProgressUpdater,
    downloaded_chunks: Arc<DashMap<String, u64>>,
    chunks_since_save: Arc<AtomicU64>,
    pending_saves: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    state_saver: StateSaver,
    adaptive_assembly: Arc<AdaptiveAssembly>,
}

pub(crate) struct InstallerData {
    client: Arc<Client>,
    chunk_download: Arc<DownloadInfo>,
    files: Vec<SophonManifestAssetProperty>,
    label: String,
    pub matching_field: String,
}

struct DownloadItem {
    chunk: SophonManifestAssetChunk,
    client: Arc<Client>,
    chunk_download: Arc<DownloadInfo>,
    is_pre_downloaded: bool,
}

type PendingCount = Arc<Mutex<usize>>;
type FileEntry = (usize, usize, PendingCount);

pub struct SophonInstaller {
    pub client: Client,
    pub manifest: SophonManifestProto,
    pub chunk_download: DownloadInfo,
    pub label: String,
    pub matching_field: String,
    #[allow(dead_code)]
    pub tag: String,
    pub manifest_hash: String,
}

impl SophonInstaller {
    pub async fn from_manifest_meta(
        client: &Client,
        meta: &SophonManifestMeta,
        tag: &str,
    ) -> SophonResult<Self> {
        let result =
            super::api::fetch_manifest(client, &meta.manifest_download, &meta.manifest.id).await?;
        Ok(Self {
            client: client.clone(),
            manifest: result.manifest,
            chunk_download: meta.chunk_download.clone(),
            label: meta
                .chunk_download
                .url_suffix
                .trim_matches('/')
                .replace('/', "-"),
            matching_field: meta.matching_field.clone(),
            tag: tag.to_owned(),
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

    let build = fetch_build(client, &branch.main, None).await?;
    let tag = build.tag.clone();

    let installers = build_installers_from_data(client, &build, vo_lang, &tag).await?;
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
        fetch_build(client, &branch.main, Some(from_tag)),
        fetch_build(client, &branch.main, None),
    )?;

    let new_tag = new_build.tag.clone();
    let (installers, deleted_files) =
        build_diff_installers(client, &old_build, &new_build, vo_lang, &new_tag).await?;
    let manifest_hash = combine_manifest_hashes(&installers);
    Ok((installers, deleted_files, new_tag, manifest_hash))
}

pub async fn build_preinstall_installers(
    client: &Client,
    game_id: &str,
    vo_lang: &str,
) -> SophonResult<(Vec<SophonInstaller>, String, String)> {
    let (_, pre_branch) = fetch_front_door(client, game_id).await?;
    let pre_branch = pre_branch.ok_or(SophonError::NoPreinstallAvailable)?;

    let build = fetch_build(client, &pre_branch, None).await?;
    let tag = build.tag.clone();

    let installers = build_installers_from_data(client, &build, vo_lang, &tag).await?;
    let manifest_hash = combine_manifest_hashes(&installers);
    Ok((installers, tag, manifest_hash))
}

async fn build_installers_from_data(
    client: &Client,
    build: &SophonBuildData,
    vo_lang: &str,
    tag: &str,
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

    let mut installers = Vec::with_capacity(qualifying.len());
    for meta in &qualifying {
        installers.push(SophonInstaller::from_manifest_meta(client, meta, tag).await?);
    }

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
                return false;
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
    tag: &str,
) -> SophonResult<(Vec<SophonInstaller>, Vec<String>)> {
    let old_by_field: HashMap<&str, &SophonManifestMeta> = old_build
        .manifests
        .iter()
        .map(|m| (m.matching_field.as_str(), m))
        .collect();

    let mut installers = Vec::new();
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

        let old_md5_map: HashMap<String, String> =
            match old_by_field.get(new_meta.matching_field.as_str()) {
                Some(old_meta) => {
                    let old_result = super::api::fetch_manifest(
                        client,
                        &old_meta.manifest_download,
                        &old_meta.manifest.id,
                    )
                    .await?;

                    deleted_files.extend(collect_deleted_files(&old_result.manifest, &new_names));

                    build_old_md5_map(old_result.manifest)
                }
                None => HashMap::new(),
            };

        let diff_files = compute_diff_files(new_result.manifest, &old_md5_map);

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
            tag: tag.to_owned(),
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

fn build_installer_data(installers: Vec<SophonInstaller>) -> Vec<InstallerData> {
    installers
        .into_iter()
        .map(|inst| InstallerData {
            label: inst.label,
            matching_field: inst.matching_field,
            client: Arc::new(inst.client),
            chunk_download: Arc::new(inst.chunk_download),
            files: inst
                .manifest
                .assets
                .into_iter()
                .filter(|a| !a.is_directory())
                .collect(),
        })
        .collect()
}

fn compute_totals(installer_data: &[InstallerData]) -> (u64, u64) {
    let mut seen_chunks: HashSet<&str> = HashSet::new();
    let total_compressed: u64 = installer_data
        .iter()
        .flat_map(|d| d.files.iter())
        .flat_map(|f| f.asset_chunks.iter())
        .filter(|c| seen_chunks.insert(c.chunk_name.as_str()))
        .map(|c| c.chunk_size)
        .sum();

    let total_files: u64 = installer_data.iter().map(|d| d.files.len() as u64).sum();

    (total_compressed, total_files)
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn register_chunks_for_file(
    file: &SophonManifestAssetProperty,
    file_idx: usize,
    tmp_dir_idx: usize,
    ctx: &InstallContext,
    chunk_to_files: &DashMap<String, Vec<FileEntry>>,
    download_items: &mut Vec<DownloadItem>,
    data: &InstallerData,
    pre_downloaded: &HashSet<String>,
) {
    let chunk_count = file.asset_chunks.len();
    if chunk_count == 0 {
        return;
    }

    let pending = Arc::new(Mutex::new(chunk_count));
    for chunk in &file.asset_chunks {
        chunk_to_files
            .entry(chunk.chunk_name.clone())
            .or_default()
            .push((file_idx, tmp_dir_idx, Arc::clone(&pending)));

        let is_pre = pre_downloaded.contains(&chunk.chunk_name);

        match ctx.chunk_refcounts.entry(chunk.chunk_name.clone()) {
            Entry::Vacant(vacant) => {
                vacant.insert(1);
                download_items.push(DownloadItem {
                    chunk: chunk.clone(),
                    client: Arc::clone(&data.client),
                    chunk_download: Arc::clone(&data.chunk_download),
                    is_pre_downloaded: is_pre,
                });
            }
            Entry::Occupied(mut occupied) => {
                *occupied.get_mut() += 1;
                if is_pre
                    && let Some(item) = download_items
                        .iter_mut()
                        .find(|i| i.chunk.chunk_name == chunk.chunk_name)
                {
                    item.is_pre_downloaded = is_pre;
                }
            }
        }
    }
}

async fn build_download_state(
    installer_data: Vec<InstallerData>,
    ctx: &InstallContext,
    assemble_tx: &mpsc::Sender<(usize, usize)>,
    completed_indices: Option<&HashSet<usize>>,
    pre_downloaded: &HashSet<String>,
) -> SophonResult<(Vec<DownloadItem>, Arc<DashMap<String, Vec<FileEntry>>>)> {
    let chunk_to_files: Arc<DashMap<String, Vec<FileEntry>>> = Arc::new(DashMap::new());
    let mut download_items: Vec<DownloadItem> = Vec::new();
    let mut file_idx = 0usize;

    for (tmp_dir_idx, data) in installer_data.into_iter().enumerate() {
        let needs_tmp_dir = data
            .files
            .iter()
            .enumerate()
            .any(|(i, _)| completed_indices.is_none_or(|set| !set.contains(&(file_idx + i))));
        if needs_tmp_dir {
            let tmp_dir = &ctx.all_tmp_dirs[tmp_dir_idx];
            let td = tmp_dir.clone();
            tokio::task::spawn_blocking(move || fs::create_dir_all(&td))
                .await?
                .map_err(SophonError::from)?;
        }

        for _ in 0..data.files.len() {
            if completed_indices.is_some_and(|set| set.contains(&file_idx)) {
                file_idx += 1;
                continue;
            }

            let file = &ctx.all_files[file_idx];
            let chunk_count = file.asset_chunks.len();
            if chunk_count == 0 {
                let _ = assemble_tx.send((file_idx, tmp_dir_idx)).await;
                file_idx += 1;
                continue;
            }

            register_chunks_for_file(
                file,
                file_idx,
                tmp_dir_idx,
                ctx,
                &chunk_to_files,
                &mut download_items,
                &data,
                pre_downloaded,
            );
            file_idx += 1;
        }
    }

    log::info!(
        "build_download_state: {} download items, {} chunk->file mappings, file_idx={}",
        download_items.len(),
        chunk_to_files.len(),
        file_idx,
    );
    Ok((download_items, chunk_to_files))
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

async fn drain_join_set(
    join_set: &mut tokio::task::JoinSet<Result<SophonResult<()>, tokio::task::JoinError>>,
) -> SophonResult<()> {
    while let Some(res) = join_set.join_next().await {
        let _ = res??;
    }
    Ok(())
}

fn spawn_assembly_coordinator(
    ctx: &Arc<InstallContext>,
    assemble_rx: mpsc::Receiver<(usize, usize)>,
) -> (
    tokio::task::JoinHandle<SophonResult<()>>,
    tokio_util::sync::CancellationToken,
) {
    let ctx = Arc::clone(ctx);
    let assembly_cancel = ctx.adaptive_assembly.spawn_adjuster();

    let handle = tokio::spawn(async move {
        let mut rx = assemble_rx;
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
                match rx.recv().await {
                    Some((file_idx, tmp_dir_idx)) => {
                        let params = make_assembly_params(&ctx, file_idx, tmp_dir_idx);
                        let updater = Arc::clone(&ctx.updater);
                        join_set.spawn(spawn_assembly_task(params, move |p| updater(p)));
                    }
                    None => return drain_join_set(&mut join_set).await,
                }
            } else if let Some(res) = join_set.join_next().await {
                let _ = res??;
            }
        }
    });

    (handle, assembly_cancel)
}

fn spawn_adaptive_adjuster(adaptive: &Arc<AdaptiveSemaphore>) -> CancellationToken {
    let cancel_token = CancellationToken::new();
    let adaptive = Arc::clone(adaptive);
    let token = cancel_token.clone();

    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(ADAPTIVE_WINDOW_SECS));
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
    dest: PathBuf,
    chunk: &SophonManifestAssetChunk,
    game_dir: &Path,
    verify_cache: &Arc<DashMap<String, VerificationEntry>>,
) -> SophonResult<bool> {
    if !dest.exists() {
        return Ok(true);
    }

    let chunk_size = chunk.chunk_size;
    let expected_md5 = chunk.chunk_compressed_hash_md5.clone();
    let cache = Arc::clone(verify_cache);
    let gd = game_dir.to_path_buf();

    let valid = tokio::task::spawn_blocking(move || {
        cache::check_file_md5_cached(&dest, chunk_size, &expected_md5, &gd, &cache).unwrap_or(false)
    })
    .await?;

    Ok(!valid)
}

async fn download_chunk_with_retries(
    item: &DownloadItem,
    dest: &Path,
    ctx: &InstallContext,
    handle: &DownloadHandle,
) -> SophonResult<()> {
    let mut last_err = String::new();
    let mut success = false;

    for attempt in 0..MAX_RETRIES {
        if handle.is_cancelled() {
            return Err(SophonError::Cancelled);
        }

        match super::download::download_chunk(&item.client, &item.chunk_download, &item.chunk, dest)
            .await
        {
            Ok(()) => {
                success = true;
                break;
            }
            Err(e) => {
                last_err = e.to_string();
                if attempt < MAX_RETRIES - 1 {
                    (ctx.updater)(SophonProgress::Warning {
                        message: format!(
                            "Chunk {} failed (attempt {}/{}): {last_err}",
                            item.chunk.chunk_name,
                            attempt + 1,
                            MAX_RETRIES
                        ),
                    });
                }
                let _ = fs::remove_file(dest);
            }
        }
    }

    if !success {
        return Err(SophonError::DownloadFailed {
            chunk: item.chunk.chunk_name.clone(),
            attempts: MAX_RETRIES,
            error: last_err,
        });
    }

    Ok(())
}

async fn notify_assembly_ready(
    chunk_name: &str,
    chunk_to_files: &DashMap<String, Vec<FileEntry>>,
    assemble_tx: &mpsc::Sender<(usize, usize)>,
) {
    let ready: Vec<(usize, usize)> = match chunk_to_files.remove(chunk_name) {
        Some((_, entries)) => entries
            .into_iter()
            .filter_map(|(file_idx, tmp_dir_idx, pending)| {
                let mut count = pending.lock().unwrap();
                *count -= 1;
                if *count == 0 {
                    Some((file_idx, tmp_dir_idx))
                } else {
                    None
                }
            })
            .collect(),
        None => {
            log::warn!(
                "notify_assembly_ready: chunk '{}' not found in chunk_to_files (already removed or never registered)",
                chunk_name
            );
            Vec::new()
        }
    };

    for entry in ready {
        let _ = assemble_tx.send(entry).await;
    }
}

async fn process_download_item(
    item: DownloadItem,
    ctx: Arc<InstallContext>,
    chunk_to_files: Arc<DashMap<String, Vec<FileEntry>>>,
    assemble_tx: mpsc::Sender<(usize, usize)>,
    handle: DownloadHandle,
    adaptive: Arc<AdaptiveSemaphore>,
) -> SophonResult<()> {
    let _permit = adaptive.acquire().await;

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

    let dest = ctx.chunks_dir.join(assembly::chunk_filename(&item.chunk));

    let mut was_actually_downloaded = false;
    let needs_download =
        check_needs_download(dest.clone(), &item.chunk, &ctx.game_dir, &ctx.verify_cache).await?;
    if needs_download {
        download_chunk_with_retries(&item, &dest, &ctx, &handle).await?;
        was_actually_downloaded = true;
    }

    if was_actually_downloaded && item.is_pre_downloaded {
        ctx.resume_bytes_offset
            .fetch_sub(item.chunk.chunk_size, Ordering::Relaxed);
    }

    ctx.downloaded_chunks
        .insert(item.chunk.chunk_name.clone(), item.chunk.chunk_size);

    let count = ctx.chunks_since_save.fetch_add(1, Ordering::Relaxed) + 1;
    if count.is_multiple_of(crate::commands::sophon_downloader::CHUNK_STATE_SAVE_INTERVAL) {
        let dc = Arc::clone(&ctx.downloaded_chunks);
        let saver = Arc::clone(&ctx.state_saver);
        let handle = tokio::task::spawn_blocking(move || saver(&dc));
        let mut pending = ctx.pending_saves.lock().unwrap_or_else(|e| {
            log::error!("pending_saves mutex poisoned, recovering");
            e.into_inner()
        });
        pending.retain(|h| !h.is_finished());
        pending.push(handle);
    }

    let db = if was_actually_downloaded || !item.is_pre_downloaded {
        ctx.downloaded_bytes
            .fetch_add(item.chunk.chunk_size, Ordering::Relaxed)
            + item.chunk.chunk_size
    } else {
        ctx.downloaded_bytes.load(Ordering::Relaxed)
    };

    if was_actually_downloaded || !item.is_pre_downloaded {
        adaptive.record_bytes(item.chunk.chunk_size);
    }

    {
        let mut lu = ctx.last_update.lock().unwrap();
        if lu.elapsed() >= std::time::Duration::from_millis(PROGRESS_UPDATE_INTERVAL_MS) {
            let elapsed_secs = ctx.download_start.elapsed().as_secs_f64();
            let speed_bps = if elapsed_secs > 0.0 {
                db as f64 / elapsed_secs
            } else {
                0.0
            };
            let remaining_bytes = ctx
                .total_bytes
                .saturating_sub(db + ctx.resume_bytes_offset.load(Ordering::Relaxed));
            let eta_seconds = if speed_bps > 0.0 {
                remaining_bytes as f64 / speed_bps
            } else {
                0.0
            };
            (ctx.updater)(SophonProgress::Downloading {
                downloaded_bytes: db + ctx.resume_bytes_offset.load(Ordering::Relaxed),
                total_bytes: ctx.total_bytes,
                speed_bps,
                eta_seconds,
            });
            *lu = Instant::now();
        }
    }

    notify_assembly_ready(&item.chunk.chunk_name, &chunk_to_files, &assemble_tx).await;

    Ok(())
}

async fn run_downloads(
    ctx: Arc<InstallContext>,
    download_items: Vec<DownloadItem>,
    chunk_to_files: Arc<DashMap<String, Vec<FileEntry>>>,
    assemble_tx: &mpsc::Sender<(usize, usize)>,
    handle: DownloadHandle,
    adaptive: Arc<AdaptiveSemaphore>,
) -> Vec<SophonResult<()>> {
    futures_util::stream::iter(download_items)
        .map(|item| {
            let ctx = Arc::clone(&ctx);
            let chunk_to_files = Arc::clone(&chunk_to_files);
            let assemble_tx = assemble_tx.clone();
            let handle = handle.clone();
            let adaptive = Arc::clone(&adaptive);

            process_download_item(item, ctx, chunk_to_files, assemble_tx, handle, adaptive)
        })
        .buffer_unordered(ADAPTIVE_MAX_CONCURRENCY)
        .collect()
        .await
}

#[allow(clippy::too_many_arguments)]
async fn finalize_install(
    ctx: &InstallContext,
    results: Vec<SophonResult<()>>,
    deleted_files: &[String],
    tag: &str,
    is_preinstall: bool,
    assembly_task: tokio::task::JoinHandle<SophonResult<()>>,
    game_code: &str,
    vo_langs: &[String],
) -> SophonResult<()> {
    let cancelled = results
        .iter()
        .any(|r| matches!(r, Err(SophonError::Cancelled)));
    if cancelled {
        let cd = Arc::clone(&ctx.chunks_dir);
        let _ = tokio::task::spawn_blocking(move || {
            let _ = fs::remove_dir_all(&*cd);
        })
        .await;
        let _ = assembly_task.await;
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
            log::info!("Sophon install: all {} files assembled successfully", total);
        }
    }

    {
        let dc = Arc::clone(&ctx.downloaded_chunks);
        let saver = Arc::clone(&ctx.state_saver);
        tokio::task::spawn_blocking(move || saver(&dc))
            .await
            .unwrap_or_else(|e| {
                log::error!("Final state save join error: {e}");
            });
    }

    {
        let _ = cache::save_verification_cache(&ctx.game_dir, &ctx.verify_cache);
    }

    if !deleted_files.is_empty() {
        let gd = ctx.game_dir.clone();
        let df = deleted_files.to_vec();
        tokio::task::spawn_blocking(move || {
            for rel in &df {
                let _ = fs::remove_file(gd.join(rel));
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
            if let Err(e) = super::game_filters::write_hkrpg_audio_lang_record(&gd, &vl) {
                log::warn!("Failed to write hkrpg audio language record: {}", e);
            }
        })
        .await?;
        let gd = ctx.game_dir.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = super::game_filters::write_hkrpg_app_info(&gd) {
                log::warn!("Failed to write hkrpg app.info: {}", e);
            }
            if let Err(e) = super::game_filters::write_hkrpg_binary_version_files(&gd) {
                log::warn!("Failed to write hkrpg binary version files: {}", e);
            }
        })
        .await?;
    } else if game_code == "hk4e" && !is_preinstall {
        let gd = ctx.game_dir.clone();
        let vl = vo_langs.to_vec();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = super::game_filters::write_hk4e_audio_lang_record(&gd, &vl) {
                log::warn!("Failed to write hk4e audio language record: {}", e);
            }
        })
        .await?;
        let gd = ctx.game_dir.clone();
        let vl = vo_langs.to_vec();
        let af = (*ctx.all_files).clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = super::game_filters::write_pkg_version_from_manifest(&gd, &af, &vl) {
                log::warn!("Failed to write hk4e pkg_version: {}", e);
            }
        })
        .await?;
    } else if game_code == "nap" && !is_preinstall {
        let gd = ctx.game_dir.clone();
        let vl = vo_langs.to_vec();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = super::game_filters::write_nap_audio_lang_records(&gd, &vl) {
                log::warn!("Failed to write nap audio language records: {}", e);
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

    let ResumeContext {
        prev_manifest_hash,
        mut prev_downloaded_chunks,
    } = resume;
    if options.is_resume {
        // Validate that chunk files referenced in persisted state actually exist on
        // disk. Stale entries (e.g. user deleted game files between sessions)
        // would otherwise inflate the resume offset, causing incorrect progress
        // and skipped downloads.
        {
            let chunks_dir_validate = Arc::clone(&chunks_dir);
            prev_downloaded_chunks = tokio::task::spawn_blocking(move || {
                let before = prev_downloaded_chunks.len();
                prev_downloaded_chunks.retain(|chunk_name, _| {
                    chunks_dir_validate
                        .join(format!("{}.zstd", chunk_name))
                        .exists()
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
        let current_manifest_hash = combine_manifest_hashes(&installers);
        if prev_manifest_hash != current_manifest_hash {
            log::warn!(
                "Manifest changed on resume (old={}, new={}), re-verifying all chunks",
                prev_manifest_hash,
                current_manifest_hash
            );
        } else {
            log::info!(
                "Manifest unchanged on resume (hash={}), preserving {} cached chunks",
                current_manifest_hash,
                prev_downloaded_chunks.len()
            );
        }
    }

    let mut installer_data = build_installer_data(installers);
    if game_code == "nap" {
        super::game_filters::filter_nap_installers(game_dir, &mut installer_data);
    }
    let mut all_files: Vec<SophonManifestAssetProperty> = installer_data
        .iter()
        .flat_map(|d| d.files.clone())
        .collect();
    if game_code == "hkrpg" {
        super::game_filters::filter_hkrpg_asset_list(game_dir, &mut all_files);
    } else if game_code == "hk4e" {
        super::game_filters::filter_hk4e_asset_list(game_dir, &mut all_files, vo_langs);
    } else if game_code == "nap" {
        super::game_filters::filter_nap_asset_list(game_dir, &mut all_files);
    }
    let filtered_set: HashSet<String> = all_files.iter().map(|f| f.asset_name.clone()).collect();
    let installer_data: Vec<InstallerData> = installer_data
        .into_iter()
        .map(|mut d| {
            d.files.retain(|f| filtered_set.contains(&f.asset_name));
            d
        })
        .collect();
    let all_files: Arc<Vec<SophonManifestAssetProperty>> = Arc::new(all_files);
    let all_tmp_dirs: Arc<Vec<std::path::PathBuf>> = Arc::new(
        installer_data
            .iter()
            .map(|d| game_dir.join(format!("tmp-{}", d.label)))
            .collect(),
    );

    let (total_compressed, total_files) = compute_totals(&installer_data);
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
            d.files.len(),
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
    let mut completed_chunk_names: HashSet<&str> = HashSet::new();
    let completed_indices = if options.is_resume {
        let total = all_files.len() as u64;
        let mut last_calc_update = Instant::now();
        (callbacks.updater)(SophonProgress::CalculatingDownloads {
            checked_files: 0,
            total_files: total,
        });
        let mut indices = HashSet::new();
        for (file_idx, file) in all_files.iter().enumerate() {
            if file.asset_chunks.is_empty() {
                indices.insert(file_idx);
                pre_assembled += 1;
            } else {
                let target_path = game_dir.join(&file.asset_name);
                if target_path.exists() {
                    let valid = {
                        let tp = target_path.clone();
                        let sz = file.asset_size;
                        let md5 = file.asset_hash_md5.clone();
                        let vc = Arc::clone(&verify_cache);
                        let gd = game_dir.to_path_buf();
                        tokio::task::spawn_blocking(move || {
                            cache::check_file_md5_cached(&tp, sz, &md5, &gd, &vc).unwrap_or(false)
                        })
                        .await?
                    };
                    if valid {
                        indices.insert(file_idx);
                        let file_chunk_size: u64 =
                            file.asset_chunks.iter().map(|c| c.chunk_size).sum();
                        resume_bytes_offset += file_chunk_size;
                        for c in &file.asset_chunks {
                            completed_chunk_names.insert(&c.chunk_name);
                        }
                        pre_assembled += 1;
                    } else {
                        let tp = target_path.clone();
                        tokio::task::spawn_blocking(move || {
                            let _ = fs::remove_file(tp);
                        })
                        .await?;
                    }
                }
            }
            let checked = (file_idx + 1) as u64;
            if last_calc_update.elapsed()
                >= std::time::Duration::from_millis(PROGRESS_UPDATE_INTERVAL_MS)
            {
                (callbacks.updater)(SophonProgress::CalculatingDownloads {
                    checked_files: checked,
                    total_files: total,
                });
                last_calc_update = Instant::now();
            }
        }
        (callbacks.updater)(SophonProgress::CalculatingDownloads {
            checked_files: total,
            total_files: total,
        });
        Some(indices)
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
    let initial_dashmap: DashMap<String, u64> = DashMap::new();
    for (k, v) in initial_chunks {
        initial_dashmap.insert(k, v);
    }
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
        last_update: Arc::new(Mutex::new(Instant::now())),
        download_start: Instant::now(),
        updater: Arc::clone(&callbacks.updater),
        downloaded_chunks: Arc::new(initial_dashmap),
        chunks_since_save: Arc::new(AtomicU64::new(0)),
        pending_saves: Arc::new(Mutex::new(Vec::new())),
        state_saver: callbacks.state_saver,
        adaptive_assembly: Arc::clone(&adaptive_assembly),
    });

    let (assemble_tx, assemble_rx) = mpsc::channel::<(usize, usize)>(ASSEMBLY_CHANNEL_SIZE);
    let (assembly_task, _assembly_cancel_token) = spawn_assembly_coordinator(&ctx, assemble_rx);

    let (download_items, chunk_to_files) = build_download_state(
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
        *ctx.last_update.lock().unwrap_or_else(|e| {
            log::error!("last_update mutex poisoned, recovering");
            e.into_inner()
        }) = Instant::now();
    }

    let adaptive = Arc::new(AdaptiveSemaphore::new());
    let _cancel_token = spawn_adaptive_adjuster(&adaptive);

    let results = run_downloads(
        Arc::clone(&ctx),
        download_items,
        chunk_to_files,
        &assemble_tx,
        options.handle,
        Arc::clone(&adaptive),
    )
    .await;

    let pending_handles: Vec<tokio::task::JoinHandle<()>> = {
        let mut pending = ctx.pending_saves.lock().unwrap_or_else(|e| {
            log::error!("pending_saves mutex poisoned, recovering");
            e.into_inner()
        });
        pending.drain(..).collect()
    };

    for handle in pending_handles {
        let _ = handle.await;
    }

    drop(assemble_tx);
    finalize_install(
        &ctx,
        results,
        &deleted_files,
        tag,
        options.is_preinstall,
        assembly_task,
        game_code,
        vo_langs,
    )
    .await
}

pub async fn apply_preinstall(game_dir: &Path, preinstall_tag: &str) -> SophonResult<()> {
    let marker = game_dir.join(format!(".sophon_preinstall_{preinstall_tag}"));
    if !marker.exists() {
        return Err(SophonError::PreinstallMarkerNotFound(preinstall_tag.into()));
    }
    let gd = game_dir.to_path_buf();
    let tag = preinstall_tag.to_owned();
    tokio::task::spawn_blocking(move || {
        write_installed_tag(&gd, &tag)?;
        fs::remove_file(gd.join(format!(".sophon_preinstall_{tag}"))).map_err(SophonError::from)
    })
    .await?
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
    let build = api::fetch_build(client, &branch.main, Some(&tag)).await?;

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

    let mut manifest_results: Vec<SophonManifestProto> = Vec::new();
    let mut chunk_downloads: Vec<&DownloadInfo> = Vec::new();
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
    let mut error_count = 0u64;
    let verify_cache = cache::load_verification_cache(game_dir);
    let chunks_dir = game_dir.join("chunks");
    let mut last_emit = Instant::now();

    for (scanned, (asset, chunk_download)) in all_assets.into_iter().enumerate() {
        let scanned = (scanned + 1) as u64;
        let file_path = game_dir.join(&asset.asset_name);

        let is_valid = tokio::task::spawn_blocking({
            let verify_cache = Arc::new(verify_cache.clone());
            let file_path = file_path.clone();
            let asset_size = asset.asset_size;
            let asset_md5 = asset.asset_hash_md5.clone();
            let gd = game_dir.to_path_buf();
            move || {
                cache::check_file_md5_cached(&file_path, asset_size, &asset_md5, &gd, &verify_cache)
                    .unwrap_or(false)
            }
        })
        .await?;

        if !is_valid {
            error_count += 1;
            emit(SophonProgress::Warning {
                message: format!(
                    "File {} failed integrity check, re-downloading",
                    asset.asset_name
                ),
            });

            if let Err(e) = redownload_asset(
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
                emit(SophonProgress::Error {
                    message: format!("Failed to re-download {}: {}", asset.asset_name, e),
                });
            }
        }

        if last_emit.elapsed() >= Duration::from_millis(PROGRESS_UPDATE_INTERVAL_MS) {
            emit(SophonProgress::Verifying {
                scanned_files: scanned,
                total_files,
                error_count,
            });
            last_emit = Instant::now();
        }
    }

    emit(SophonProgress::Verifying {
        scanned_files: total_files,
        total_files,
        error_count,
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
        let chunk_path = chunks_dir.join(assembly::chunk_filename(chunk));
        let needs_download = !chunk_path.exists()
            || !cache::check_file_md5_cached(
                &chunk_path,
                chunk.chunk_size,
                &chunk.chunk_compressed_hash_md5,
                game_dir,
                &DashMap::new(),
            )
            .unwrap_or(false);

        if needs_download {
            emit(SophonProgress::Warning {
                message: format!("Re-downloading chunk {}", chunk.chunk_name),
            });
            download::download_chunk(client, chunk_download, chunk, &chunk_path).await?;
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
            files,
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
            tag: "1.0".into(),
            manifest_hash: hash.into(),
        }
    }

    #[test]
    fn compute_totals_no_dupes() {
        let data = vec![
            make_installer_data(vec![make_file(
                "a.pak",
                "aa",
                vec![make_chunk("c1", 100), make_chunk("c2", 200)],
            )]),
            make_installer_data(vec![make_file("b.pak", "bb", vec![make_chunk("c3", 300)])]),
        ];
        let (bytes, files) = compute_totals(&data);
        assert_eq!(bytes, 600);
        assert_eq!(files, 2);
    }

    #[test]
    fn compute_totals_with_dedup() {
        let data = vec![
            make_installer_data(vec![make_file(
                "a.pak",
                "aa",
                vec![make_chunk("shared", 500)],
            )]),
            make_installer_data(vec![make_file(
                "b.pak",
                "bb",
                vec![make_chunk("shared", 500)],
            )]),
        ];
        let (bytes, files) = compute_totals(&data);
        assert_eq!(bytes, 500);
        assert_eq!(files, 2);
    }

    #[test]
    fn compute_totals_empty() {
        let data: Vec<InstallerData> = vec![];
        let (bytes, files) = compute_totals(&data);
        assert_eq!(bytes, 0);
        assert_eq!(files, 0);
    }

    #[test]
    fn compute_totals_same_name_different_size() {
        let data = vec![
            make_installer_data(vec![make_file(
                "a.pak",
                "aa",
                vec![make_chunk("shared", 500)],
            )]),
            make_installer_data(vec![make_file(
                "b.pak",
                "bb",
                vec![make_chunk("shared", 600)],
            )]),
        ];
        let (bytes, files) = compute_totals(&data);
        assert_eq!(bytes, 500);
        assert_eq!(files, 2);
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
    fn compute_diff_files_dirs_filtered() {
        let new_manifest = SophonManifestProto {
            assets: vec![make_dir("GameData"), make_file("a.pak", "aa", vec![])],
        };
        let diff = compute_diff_files(new_manifest, &HashMap::new());
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].asset_name, "a.pak");
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
        assert_eq!(diff.len(), 2);
        let names: Vec<&str> = diff.iter().map(|f| f.asset_name.as_str()).collect();
        assert!(names.contains(&"new.pak"));
        assert!(names.contains(&"changed.pak"));
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
        let chunk_to_files: DashMap<String, Vec<FileEntry>> = DashMap::new();
        let (tx, mut rx) = mpsc::channel::<(usize, usize)>(16);

        let pending: PendingCount = Arc::new(Mutex::new(1usize));
        chunk_to_files.insert(
            "chunk_a".to_string(),
            vec![(0usize, 0usize, Arc::clone(&pending))],
        );

        notify_assembly_ready("chunk_a", &chunk_to_files, &tx).await;

        let received = rx.try_recv();
        assert!(received.is_ok(), "file should be sent to assembly channel");
        assert_eq!(received.unwrap(), (0, 0));
        assert_eq!(*pending.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn notify_assembly_ready_chunk_not_in_map() {
        let chunk_to_files: DashMap<String, Vec<FileEntry>> = DashMap::new();
        let (tx, rx) = mpsc::channel::<(usize, usize)>(16);
        drop(rx);

        notify_assembly_ready("nonexistent_chunk", &chunk_to_files, &tx).await;
    }

    #[tokio::test]
    async fn check_needs_download_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("does_not_exist.bin");
        let chunk = make_chunk("c1", 100);
        let cache = Arc::new(DashMap::new());

        let needs = check_needs_download(dest, &chunk, dir.path(), &cache)
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

        let needs = check_needs_download(file_path, &chunk, dir.path(), &cache)
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

        let chunk = SophonManifestAssetChunk {
            chunk_name: "test_retry_chunk".to_string(),
            chunk_decompressed_hash_md5: String::new(),
            chunk_on_file_offset: 0,
            chunk_size: data.len() as u64,
            chunk_size_decompressed: data.len() as u64,
            chunk_compressed_hash_xxh: 0,
            chunk_compressed_hash_md5: expected_md5,
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

        let item = DownloadItem {
            chunk,
            client: Arc::new(Client::new()),
            chunk_download: dl_info,
            is_pre_downloaded: false,
        };

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
            last_update: Arc::new(Mutex::new(Instant::now())),
            download_start: Instant::now(),
            updater: Arc::new(|_| {}),
            downloaded_chunks: Arc::new(DashMap::new()),
            chunks_since_save: Arc::new(AtomicU64::new(0)),
            pending_saves: Arc::new(Mutex::new(Vec::new())),
            state_saver: Arc::new(|_| {}),
            adaptive_assembly: Arc::new(AdaptiveAssembly::new()),
        });

        let handle = DownloadHandle::new();

        let result = download_chunk_with_retries(&item, &dest, &ctx, &handle).await;
        assert!(result.is_ok());
    }

    #[test]
    fn compute_totals_filters_directories() {
        let file1 = make_file("a.pak", "aa", vec![make_chunk("c1", 100)]);
        let dir1 = make_dir("GameData");
        let data = vec![make_installer_data(vec![dir1, file1])];
        let (bytes, files) = compute_totals(&data);
        assert_eq!(bytes, 100);
        assert_eq!(files, 2);
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
            tag: "1.0".into(),
            manifest_hash: "abc".into(),
        };

        let result = build_installer_data(vec![installer]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].files.len(), 1);
        assert_eq!(result[0].files[0].asset_name, "a.pak");
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
}
