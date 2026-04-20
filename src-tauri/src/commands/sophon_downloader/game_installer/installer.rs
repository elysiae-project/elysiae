use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use futures_util::StreamExt;
use reqwest::Client;
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

use super::adaptive::{ActiveGuard, AdaptiveConcurrency};
use super::api::{fetch_build, fetch_front_door, vo_lang_matches};
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

struct InstallContext {
    chunks_dir: Arc<PathBuf>,
    game_dir: PathBuf,
    all_tmp_dirs: Arc<Vec<std::path::PathBuf>>,
    all_files: Arc<Vec<SophonManifestAssetProperty>>,
    downloaded_bytes: Arc<AtomicU64>,
    assembled_files: Arc<AtomicU64>,
    total_bytes: u64,
    total_files: u64,
    verify_cache: Arc<DashMap<String, VerificationEntry>>,
    chunk_refcounts: Arc<DashMap<String, usize>>,
    last_assembly_update: Arc<Mutex<Instant>>,
    last_update: Arc<Mutex<Instant>>,
    download_start: Instant,
    updater: ProgressUpdater,
}

struct InstallerData {
    client: Arc<Client>,
    chunk_download: Arc<DownloadInfo>,
    files: Vec<SophonManifestAssetProperty>,
    label: String,
}

struct DownloadItem {
    chunk: SophonManifestAssetChunk,
    client: Arc<Client>,
    chunk_download: Arc<DownloadInfo>,
}

type PendingCount = Arc<Mutex<usize>>;
type FileEntry = (usize, usize, PendingCount);

pub struct SophonInstaller {
    pub client: Client,
    pub manifest: SophonManifestProto,
    pub chunk_download: DownloadInfo,
    pub label: String,
    #[allow(dead_code)]
    pub tag: String,
}

impl SophonInstaller {
    pub async fn from_manifest_meta(
        client: &Client,
        meta: &SophonManifestMeta,
        tag: &str,
    ) -> SophonResult<Self> {
        let manifest =
            super::api::fetch_manifest(client, &meta.manifest_download, &meta.manifest.id).await?;
        Ok(Self {
            client: client.clone(),
            manifest,
            chunk_download: meta.chunk_download.clone(),
            label: meta
                .chunk_download
                .url_suffix
                .trim_matches('/')
                .replace('/', "-"),
            tag: tag.to_owned(),
        })
    }
}

pub async fn build_installers(
    client: &Client,
    game_id: &str,
    vo_lang: &str,
) -> SophonResult<(Vec<SophonInstaller>, String)> {
    let (branch, _) = fetch_front_door(client, game_id).await?;

    let build = fetch_build(client, &branch.main, None).await?;
    let tag = build.tag.clone();

    let installers = build_installers_from_data(client, &build, vo_lang, &tag).await?;
    Ok((installers, tag))
}

pub async fn build_update_installers(
    client: &Client,
    game_id: &str,
    vo_lang: &str,
    from_tag: &str,
) -> SophonResult<(Vec<SophonInstaller>, Vec<String>, String)> {
    let (branch, _) = fetch_front_door(client, game_id).await?;

    let (old_build, new_build) = tokio::try_join!(
        fetch_build(client, &branch.main, Some(from_tag)),
        fetch_build(client, &branch.main, None),
    )?;

    let new_tag = new_build.tag.clone();
    let (installers, deleted_files) =
        build_diff_installers(client, &old_build, &new_build, vo_lang, &new_tag).await?;

    Ok((installers, deleted_files, new_tag))
}

pub async fn build_preinstall_installers(
    client: &Client,
    game_id: &str,
    vo_lang: &str,
) -> SophonResult<(Vec<SophonInstaller>, String)> {
    let (_, pre_branch) = fetch_front_door(client, game_id).await?;
    let pre_branch = pre_branch.ok_or(SophonError::NoPreinstallAvailable)?;

    let build = fetch_build(client, &pre_branch, None).await?;
    let tag = build.tag.clone();

    let installers = build_installers_from_data(client, &build, vo_lang, &tag).await?;
    Ok((installers, tag))
}

async fn build_installers_from_data(
    client: &Client,
    build: &SophonBuildData,
    vo_lang: &str,
    tag: &str,
) -> SophonResult<Vec<SophonInstaller>> {
    let game_meta = build.manifests.first().ok_or(SophonError::NoGameManifest)?;

    let vo_meta = build
        .manifests
        .iter()
        .find(|m| vo_lang_matches(&m.matching_field, vo_lang))
        .or_else(|| build.manifests.get(1))
        .ok_or_else(|| SophonError::NoVoiceManifest(vo_lang.into()))?;

    let (game_inst, vo_inst) = tokio::try_join!(
        SophonInstaller::from_manifest_meta(client, game_meta, tag),
        SophonInstaller::from_manifest_meta(client, vo_meta, tag),
    )?;

    Ok(vec![game_inst, vo_inst])
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
fn build_old_md5_map(
    old_manifest: SophonManifestProto,
) -> HashMap<String, String> {
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
        if new_meta.matching_field != "game" && !vo_lang_matches(&new_meta.matching_field, vo_lang)
        {
            continue;
        }

        let new_manifest =
            super::api::fetch_manifest(client, &new_meta.manifest_download, &new_meta.manifest.id)
                .await?;

        let new_names: HashSet<&str> = new_manifest
            .assets
            .iter()
            .map(|f| f.asset_name.as_str())
            .collect();

        let old_md5_map: HashMap<String, String> =
            match old_by_field.get(new_meta.matching_field.as_str()) {
                Some(old_meta) => {
                    let old_manifest = super::api::fetch_manifest(
                        client,
                        &old_meta.manifest_download,
                        &old_meta.manifest.id,
                    )
                    .await?;

                    deleted_files.extend(collect_deleted_files(&old_manifest, &new_names));

                    build_old_md5_map(old_manifest)
                }
                None => HashMap::new(),
            };

        let diff_files = compute_diff_files(new_manifest, &old_md5_map);

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
            tag: tag.to_owned(),
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
    let total_compressed: u64 = installer_data
        .iter()
        .flat_map(|d| d.files.iter())
        .flat_map(|f| f.asset_chunks.iter())
        .map(|c| c.chunk_size)
        .sum();

    let total_files: u64 = installer_data.iter().map(|d| d.files.len() as u64).sum();

    (total_compressed, total_files)
}

#[inline]
fn register_chunks_for_file(
    file: &SophonManifestAssetProperty,
    file_idx: usize,
    tmp_dir_idx: usize,
    ctx: &InstallContext,
    chunk_to_files: &DashMap<String, Vec<FileEntry>>,
    download_items: &mut Vec<DownloadItem>,
    data: &InstallerData,
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

        match ctx.chunk_refcounts.entry(chunk.chunk_name.clone()) {
            Entry::Vacant(vacant) => {
                vacant.insert(1);
                download_items.push(DownloadItem {
                    chunk: chunk.clone(),
                    client: Arc::clone(&data.client),
                    chunk_download: Arc::clone(&data.chunk_download),
                });
            }
            Entry::Occupied(mut occupied) => {
                *occupied.get_mut() += 1;
            }
        }
    }
}

async fn build_download_state(
    installer_data: Vec<InstallerData>,
    ctx: &InstallContext,
    assemble_tx: &mpsc::Sender<(usize, usize)>,
) -> SophonResult<(Vec<DownloadItem>, Arc<DashMap<String, Vec<FileEntry>>>)> {
    let chunk_to_files: Arc<DashMap<String, Vec<FileEntry>>> = Arc::new(DashMap::new());
    let mut download_items: Vec<DownloadItem> = Vec::new();
    let mut file_idx = 0usize;

    for (tmp_dir_idx, data) in installer_data.into_iter().enumerate() {
        let tmp_dir = &ctx.all_tmp_dirs[tmp_dir_idx];
        {
            let td = tmp_dir.clone();
            tokio::task::spawn_blocking(move || fs::create_dir_all(&td))
                .await?
                .map_err(SophonError::from)?;
        }

        for _ in 0..data.files.len() {
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
            );
            file_idx += 1;
        }
    }

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
) -> tokio::task::JoinHandle<SophonResult<()>> {
    let ctx = Arc::clone(ctx);

    tokio::spawn(async move {
        let mut rx = assemble_rx;
        let mut join_set = tokio::task::JoinSet::new();

        loop {
            while join_set.len() < ASSEMBLY_CONCURRENCY {
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
    })
}

fn spawn_adaptive_adjuster(adaptive: &Arc<AdaptiveConcurrency>) -> CancellationToken {
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
    verify_cache: &Arc<DashMap<String, VerificationEntry>>,
) -> SophonResult<bool> {
    if !dest.exists() {
        return Ok(true);
    }

    let chunk_size = chunk.chunk_size;
    let expected_md5 = chunk.chunk_compressed_hash_md5.clone();
    let cache = Arc::clone(verify_cache);

    let valid = tokio::task::spawn_blocking(move || {
        cache::check_file_md5_cached(&dest, chunk_size, &expected_md5, &cache).unwrap_or(false)
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
                    let _ = fs::remove_file(dest);
                }
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

fn notify_assembly_ready(
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
        None => Vec::new(),
    };

    for entry in ready {
        let _ = assemble_tx.try_send(entry);
    }
}

async fn process_download_item(
    item: DownloadItem,
    ctx: Arc<InstallContext>,
    chunk_to_files: Arc<DashMap<String, Vec<FileEntry>>>,
    assemble_tx: mpsc::Sender<(usize, usize)>,
    handle: DownloadHandle,
    adaptive: Arc<AdaptiveConcurrency>,
    semaphore: Arc<Semaphore>,
) -> SophonResult<()> {
    while !adaptive.can_start() {
        tokio::task::yield_now().await;
    }
    let _guard = ActiveGuard::new(&adaptive);
    let _permit = semaphore.acquire().await?;

    {
        let db = ctx.downloaded_bytes.load(Ordering::Relaxed);
        handle
            .wait_if_paused(&*ctx.updater, db, ctx.total_bytes)
            .await?;
    }

    let dest = ctx.chunks_dir.join(assembly::chunk_filename(&item.chunk));

    let needs_download = check_needs_download(dest.clone(), &item.chunk, &ctx.verify_cache).await?;

    if needs_download {
        download_chunk_with_retries(&item, &dest, &ctx, &handle).await?;
    }

    let db = ctx
        .downloaded_bytes
        .fetch_add(item.chunk.chunk_size, Ordering::Relaxed)
        + item.chunk.chunk_size;

    adaptive.record_bytes(item.chunk.chunk_size);

{
    let mut lu = ctx.last_update.lock().unwrap();
    if lu.elapsed() >= std::time::Duration::from_millis(PROGRESS_UPDATE_INTERVAL_MS) {
        let elapsed_secs = ctx.download_start.elapsed().as_secs_f64();
        let speed_bps = if elapsed_secs > 0.0 {
            db as f64 / elapsed_secs
        } else {
            0.0
        };
        let remaining_bytes = ctx.total_bytes.saturating_sub(db);
        let eta_seconds = if speed_bps > 0.0 {
            remaining_bytes as f64 / speed_bps
        } else {
            0.0
        };
        (ctx.updater)(SophonProgress::Downloading {
            downloaded_bytes: db,
            total_bytes: ctx.total_bytes,
            speed_bps,
            eta_seconds,
        });
        *lu = Instant::now();
    }
}

    notify_assembly_ready(&item.chunk.chunk_name, &chunk_to_files, &assemble_tx);

    Ok(())
}

async fn run_downloads(
    ctx: Arc<InstallContext>,
    download_items: Vec<DownloadItem>,
    chunk_to_files: Arc<DashMap<String, Vec<FileEntry>>>,
    assemble_tx: &mpsc::Sender<(usize, usize)>,
    handle: DownloadHandle,
    adaptive: Arc<AdaptiveConcurrency>,
    semaphore: Arc<Semaphore>,
) -> Vec<SophonResult<()>> {
    futures_util::stream::iter(download_items)
        .map(|item| {
            let ctx = Arc::clone(&ctx);
            let chunk_to_files = Arc::clone(&chunk_to_files);
            let assemble_tx = assemble_tx.clone();
            let handle = handle.clone();
            let adaptive = Arc::clone(&adaptive);
            let semaphore = Arc::clone(&semaphore);

            process_download_item(
                item,
                ctx,
                chunk_to_files,
                assemble_tx,
                handle,
                adaptive,
                semaphore,
            )
        })
        .buffer_unordered(ADAPTIVE_MAX_CONCURRENCY)
        .collect()
        .await
}

async fn finalize_install(
    ctx: &InstallContext,
    results: Vec<SophonResult<()>>,
    deleted_files: &[String],
    tag: &str,
    is_preinstall: bool,
    assembly_task: tokio::task::JoinHandle<SophonResult<()>>,
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
        (ctx.updater)(SophonProgress::Finished);
        return Ok(());
    }

    results.into_iter().find(|r| r.is_err()).transpose()?;
    assembly_task.await??;

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

    Ok(())
}

pub async fn install(
    installers: Vec<SophonInstaller>,
    game_dir: &Path,
    deleted_files: Vec<String>,
    tag: &str,
    is_preinstall: bool,
    handle: DownloadHandle,
    updater: impl Fn(SophonProgress) + Send + Sync + Clone + 'static,
) -> SophonResult<()> {
    let chunks_dir = Arc::new(game_dir.join("chunks"));
    prepare_directories(game_dir, &chunks_dir).await?;

    let installer_data = build_installer_data(installers);
    let all_files: Arc<Vec<SophonManifestAssetProperty>> = Arc::new(
        installer_data
            .iter()
            .flat_map(|d| d.files.clone())
            .collect(),
    );
    let all_tmp_dirs: Arc<Vec<std::path::PathBuf>> = Arc::new(
        installer_data
            .iter()
            .map(|d| game_dir.join(format!("tmp-{}", d.label)))
            .collect(),
    );

    let (total_compressed, total_files) = compute_totals(&installer_data);

let ctx = Arc::new(InstallContext {
    chunks_dir: Arc::clone(&chunks_dir),
    game_dir: game_dir.to_path_buf(),
    all_tmp_dirs: Arc::clone(&all_tmp_dirs),
    all_files: Arc::clone(&all_files),
    downloaded_bytes: Arc::new(AtomicU64::new(0)),
    assembled_files: Arc::new(AtomicU64::new(0)),
    total_bytes: total_compressed,
    total_files,
    verify_cache: Arc::new(cache::load_verification_cache(game_dir)),
    chunk_refcounts: Arc::new(DashMap::new()),
    last_assembly_update: Arc::new(Mutex::new(Instant::now())),
    last_update: Arc::new(Mutex::new(Instant::now())),
    download_start: Instant::now(),
    updater: Arc::new(updater.clone()),
});

    let (assemble_tx, assemble_rx) = mpsc::channel::<(usize, usize)>(ASSEMBLY_CHANNEL_SIZE);
    let assembly_task = spawn_assembly_coordinator(&ctx, assemble_rx);

    let (download_items, chunk_to_files) =
        build_download_state(installer_data, &ctx, &assemble_tx).await?;

    let adaptive = Arc::new(AdaptiveConcurrency::new());
    let semaphore = Arc::new(Semaphore::new(ADAPTIVE_MAX_CONCURRENCY));
    let _cancel_token = spawn_adaptive_adjuster(&adaptive);

    let results = run_downloads(
        Arc::clone(&ctx),
        download_items,
        chunk_to_files,
        &assemble_tx,
        handle,
        Arc::clone(&adaptive),
        semaphore,
    )
    .await;

    drop(assemble_tx);
    finalize_install(
        &ctx,
        results,
        &deleted_files,
        tag,
        is_preinstall,
        assembly_task,
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

  let game_meta = build.manifests.first().ok_or(SophonError::NoGameManifest)?;
  let vo_meta = build
    .manifests
    .iter()
    .find(|m| api::vo_lang_matches(&m.matching_field, vo_lang))
    .ok_or_else(|| SophonError::NoVoiceManifest(vo_lang.into()))?;

  let game_manifest = api::fetch_manifest(client, &game_meta.manifest_download, &game_meta.manifest.id).await?;
  let vo_manifest = api::fetch_manifest(client, &vo_meta.manifest_download, &vo_meta.manifest.id).await?;

  let all_assets: Vec<&SophonManifestAssetProperty> = game_manifest
    .assets
    .iter()
    .filter(|a| !a.is_directory())
    .chain(vo_manifest.assets.iter().filter(|a| !a.is_directory()))
    .collect();

  let total_files = all_assets.len() as u64;
  let mut scanned = 0u64;
  let mut error_count = 0u64;
  let verify_cache = cache::load_verification_cache(game_dir);
  let chunks_dir = game_dir.join("chunks");

  for asset in all_assets {
    scanned += 1;
    let file_path = game_dir.join(&asset.asset_name);

let is_valid = tokio::task::spawn_blocking({
    let verify_cache = Arc::new(verify_cache.clone());
    let file_path = file_path.clone();
    let asset_size = asset.asset_size;
    let asset_md5 = asset.asset_hash_md5.clone();
    move || {
      cache::check_file_md5_cached(&file_path, asset_size, &asset_md5, &verify_cache).unwrap_or(false)
    }
  }).await?;

    if !is_valid {
      error_count += 1;
      emit(SophonProgress::Warning {
        message: format!("File {} failed integrity check, re-downloading", asset.asset_name),
      });

      if let Err(e) = redownload_asset(client, asset, &chunks_dir, &file_path, &mut emit).await {
        emit(SophonProgress::Error {
          message: format!("Failed to re-download {}: {}", asset.asset_name, e),
        });
      }
    }

    emit(SophonProgress::Verifying {
      scanned_files: scanned,
      total_files,
      error_count,
    });
  }

  emit(SophonProgress::Finished);
  Ok(())
}

async fn redownload_asset(
  _client: &Client,
  asset: &SophonManifestAssetProperty,
  chunks_dir: &Path,
  file_path: &Path,
  emit: &mut (impl FnMut(SophonProgress) + Send + 'static),
) -> SophonResult<()> {
  let _manifest_meta = asset;

  for chunk in &asset.asset_chunks {
    let chunk_path = chunks_dir.join(assembly::chunk_filename(chunk));
    if !chunk_path.exists() || !cache::check_file_md5_cached(&chunk_path, chunk.chunk_size, &chunk.chunk_compressed_hash_md5, &DashMap::new()).unwrap_or(false) {
      emit(SophonProgress::Warning {
        message: format!("Re-downloading chunk {}", chunk.chunk_name),
      });
    }
  }

  if let Some(parent) = file_path.parent() {
    fs::create_dir_all(parent)?;
  }

  let _ = fs::remove_file(file_path);

  Ok(())
}
