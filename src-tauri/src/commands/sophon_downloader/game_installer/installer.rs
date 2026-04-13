use std::collections::HashSet;
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
use super::constants::*;
use super::handle::DownloadHandle;
use super::manifest::{
    DownloadInfo, SophonManifestAssetChunk, SophonManifestAssetProperty, SophonManifestProto,
};
use super::version::write_installed_tag;
use crate::commands::sophon_downloader::SophonProgress;
use crate::commands::sophon_downloader::api_scrape::{SophonBuildData, SophonManifestMeta};

pub struct SophonInstaller {
    pub client: Client,
    pub manifest: SophonManifestProto,
    pub chunk_download: DownloadInfo,
    pub label: String,
    #[allow(unused)]
    pub tag: String,
}

impl SophonInstaller {
    pub async fn from_manifest_meta(
        client: &Client,
        meta: &SophonManifestMeta,
        tag: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
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
) -> Result<(Vec<SophonInstaller>, String), Box<dyn std::error::Error + Send + Sync>> {
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
) -> Result<(Vec<SophonInstaller>, Vec<String>, String), Box<dyn std::error::Error + Send + Sync>> {
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
) -> Result<(Vec<SophonInstaller>, String), Box<dyn std::error::Error + Send + Sync>> {
    let (_, pre_branch) = fetch_front_door(client, game_id).await?;
    let pre_branch = pre_branch.ok_or("No preinstall available")?;

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
) -> Result<Vec<SophonInstaller>, Box<dyn std::error::Error + Send + Sync>> {
    let game_meta = build.manifests.first().ok_or("No game manifest")?;

    let vo_meta = build
        .manifests
        .iter()
        .find(|m| vo_lang_matches(&m.matching_field, vo_lang))
        .or_else(|| build.manifests.get(1))
        .ok_or("No VO manifest")?;

    let (game_inst, vo_inst) = tokio::try_join!(
        SophonInstaller::from_manifest_meta(client, game_meta, tag),
        SophonInstaller::from_manifest_meta(client, vo_meta, tag),
    )?;

    Ok(vec![game_inst, vo_inst])
}

async fn build_diff_installers(
    client: &Client,
    old_build: &SophonBuildData,
    new_build: &SophonBuildData,
    vo_lang: &str,
    tag: &str,
) -> Result<(Vec<SophonInstaller>, Vec<String>), Box<dyn std::error::Error + Send + Sync>> {
    let old_by_field: std::collections::HashMap<&str, &SophonManifestMeta> = old_build
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

        let old_md5_map: std::collections::HashMap<String, String> =
            match old_by_field.get(new_meta.matching_field.as_str()) {
                Some(old_meta) => {
                    let old_manifest = super::api::fetch_manifest(
                        client,
                        &old_meta.manifest_download,
                        &old_meta.manifest.id,
                    )
                    .await?;

                    for f in old_manifest.assets.iter().filter(|f| !f.is_directory()) {
                        if !new_names.contains(f.asset_name.as_str()) {
                            deleted_files.push(f.asset_name.clone());
                        }
                    }

                    old_manifest
                        .assets
                        .into_iter()
                        .filter(|f| !f.is_directory())
                        .map(|f| (f.asset_name, f.asset_hash_md5))
                        .collect()
                }
                None => std::collections::HashMap::new(),
            };

        let diff_files: Vec<SophonManifestAssetProperty> = new_manifest
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
            .collect();

        if diff_files.is_empty() {
            continue;
        }

        let mut proto = SophonManifestProto::default();
        proto.assets = diff_files;

        installers.push(SophonInstaller {
            client: client.clone(),
            manifest: proto,
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

pub async fn install(
    installers: Vec<SophonInstaller>,
    game_dir: &Path,
    deleted_files: Vec<String>,
    tag: &str,
    is_preinstall: bool,
    handle: DownloadHandle,
    updater: impl Fn(SophonProgress) + Send + Sync + Clone + 'static,
) -> Result<(), String> {
    let chunks_dir = Arc::new(game_dir.join("chunks"));
    {
        let cd = Arc::clone(&chunks_dir);
        tokio::task::spawn_blocking(move || fs::create_dir_all(&*cd))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
    }

    {
        let gd = game_dir.to_path_buf();
        tokio::task::spawn_blocking(move || cleanup_tmp_files(&gd))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
    }

    struct InstallerData {
        client: Arc<Client>,
        chunk_download: Arc<DownloadInfo>,
        files: Vec<SophonManifestAssetProperty>,
        label: String,
    }

    let installer_data: Vec<InstallerData> = installers
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
        .collect();

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

    let total_compressed: u64 = installer_data
        .iter()
        .flat_map(|d| d.files.iter())
        .flat_map(|f| f.asset_chunks.iter())
        .map(|c| c.chunk_size)
        .sum();

    let total_files: u64 = installer_data.iter().map(|d| d.files.len() as u64).sum();

    let downloaded_bytes = Arc::new(AtomicU64::new(0));
    let assembled_files = Arc::new(AtomicU64::new(0));
    let verify_cache: Arc<DashMap<String, VerificationEntry>> =
        Arc::new(cache::load_verification_cache(game_dir));

    let chunk_refcounts: Arc<DashMap<String, usize>> = Arc::new(DashMap::new());

    let last_assembly_update: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));

    let (assemble_tx, assemble_rx) = mpsc::channel::<(usize, usize)>(ASSEMBLY_CHANNEL_SIZE);

    let assembly_task = {
        let chunks_dir = Arc::clone(&chunks_dir);
        let game_dir = game_dir.to_path_buf();
        let chunk_refcounts = Arc::clone(&chunk_refcounts);
        let assembled_files = Arc::clone(&assembled_files);
        let verify_cache = Arc::clone(&verify_cache);
        let updater = updater.clone();
        let all_files = Arc::clone(&all_files);
        let all_tmp_dirs = Arc::clone(&all_tmp_dirs);
        let last_assembly_update = Arc::clone(&last_assembly_update);

        tokio::spawn(async move {
            let mut rx = assemble_rx;
            let mut join_set = tokio::task::JoinSet::new();

            loop {
                while join_set.len() < ASSEMBLY_CONCURRENCY {
                    match rx.try_recv() {
                        Ok((file_idx, tmp_dir_idx)) => {
                            let params = AssemblyTaskParams {
                                file_idx,
                                tmp_dir_idx,
                                all_files: Arc::clone(&all_files),
                                all_tmp_dirs: Arc::clone(&all_tmp_dirs),
                                game_dir: game_dir.clone(),
                                chunks_dir: Arc::clone(&chunks_dir),
                                chunk_refcounts: Arc::clone(&chunk_refcounts),
                                verify_cache: Arc::clone(&verify_cache),
                                assembled_files: Arc::clone(&assembled_files),
                                last_assembly_update: Arc::clone(&last_assembly_update),
                                total_files,
                            };
                            let updater = updater.clone();
                            join_set.spawn(spawn_assembly_task(params, updater));
                        }
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            while let Some(res) = join_set.join_next().await {
                                let _ = res.map_err(|e| e.to_string())?;
                            }
                            return Ok::<(), String>(());
                        }
                    }
                }

                if join_set.is_empty() {
                    match rx.recv().await {
                        Some((file_idx, tmp_dir_idx)) => {
                            let params = AssemblyTaskParams {
                                file_idx,
                                tmp_dir_idx,
                                all_files: Arc::clone(&all_files),
                                all_tmp_dirs: Arc::clone(&all_tmp_dirs),
                                game_dir: game_dir.clone(),
                                chunks_dir: Arc::clone(&chunks_dir),
                                chunk_refcounts: Arc::clone(&chunk_refcounts),
                                verify_cache: Arc::clone(&verify_cache),
                                assembled_files: Arc::clone(&assembled_files),
                                last_assembly_update: Arc::clone(&last_assembly_update),
                                total_files,
                            };
                            let updater = updater.clone();
                            join_set.spawn(spawn_assembly_task(params, updater));
                        }
                        None => {
                            while let Some(res) = join_set.join_next().await {
                                let _ = res.map_err(|e| e.to_string())?;
                            }
                            return Ok::<(), String>(());
                        }
                    }
                } else if let Some(res) = join_set.join_next().await {
                    let _ = res.map_err(|e| e.to_string())?;
                }
            }
        })
    };

    type PendingCount = Arc<Mutex<usize>>;
    type FileEntry = (usize, usize, PendingCount);

    struct DownloadItem {
        chunk: SophonManifestAssetChunk,
        client: Arc<Client>,
        chunk_download: Arc<DownloadInfo>,
    }

    let chunk_to_files: Arc<DashMap<String, Vec<FileEntry>>> = Arc::new(DashMap::new());
    let mut download_items: Vec<DownloadItem> = Vec::new();

    let mut file_idx = 0usize;
    for (tmp_dir_idx, data) in installer_data.into_iter().enumerate() {
        let tmp_dir = &all_tmp_dirs[tmp_dir_idx];
        {
            let td = tmp_dir.clone();
            tokio::task::spawn_blocking(move || fs::create_dir_all(&td))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
        }

        for _ in 0..data.files.len() {
            let file = &all_files[file_idx];
            let chunk_count = file.asset_chunks.len();
            if chunk_count == 0 {
                let _ = assemble_tx.send((file_idx, tmp_dir_idx)).await;
                file_idx += 1;
                continue;
            }

            let pending = Arc::new(Mutex::new(chunk_count));
            for chunk in &file.asset_chunks {
                chunk_to_files
                    .entry(chunk.chunk_name.clone())
                    .or_default()
                    .push((file_idx, tmp_dir_idx, Arc::clone(&pending)));

                match chunk_refcounts.entry(chunk.chunk_name.clone()) {
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
            file_idx += 1;
        }
    }

    let adaptive = Arc::new(AdaptiveConcurrency::new());
    let semaphore = Arc::new(Semaphore::new(ADAPTIVE_MAX_CONCURRENCY));
    let cancel_token = CancellationToken::new();

    {
        let adaptive = Arc::clone(&adaptive);
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
    }

    let last_update: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));

    let results: Vec<Result<(), String>> = futures_util::stream::iter(download_items)
        .map(|item| {
            let chunks_dir = Arc::clone(&chunks_dir);
            let downloaded_bytes = Arc::clone(&downloaded_bytes);
            let chunk_to_files = Arc::clone(&chunk_to_files);
            let assemble_tx = assemble_tx.clone();
            let handle = handle.clone();
            let updater = updater.clone();
            let last_update = Arc::clone(&last_update);
            let verify_cache = Arc::clone(&verify_cache);
            let adaptive = Arc::clone(&adaptive);
            let semaphore = Arc::clone(&semaphore);

            async move {
                while !adaptive.can_start() {
                    tokio::task::yield_now().await;
                }
                let _guard = ActiveGuard::new(&adaptive);
                let _permit = semaphore.acquire().await.map_err(|e| format!("{e}"))?;

                {
                    let db = downloaded_bytes.load(Ordering::Relaxed);
                    handle
                        .wait_if_paused(&updater, db, total_compressed)
                        .await?;
                }

                let dest = chunks_dir.join(assembly::chunk_filename(&item.chunk));

                let needs_download = if dest.exists() {
                    let dest_check = dest.clone();
                    let chunk_size = item.chunk.chunk_size;
                    let expected_md5 = item.chunk.chunk_compressed_hash_md5.clone();
                    let cache = Arc::clone(&verify_cache);
                    !tokio::task::spawn_blocking(move || {
                        cache::check_file_md5_cached(&dest_check, chunk_size, &expected_md5, &cache)
                            .unwrap_or(false)
                    })
                    .await
                    .map_err(|e| e.to_string())?
                } else {
                    true
                };

                if needs_download {
                    let mut last_err = String::new();
                    let mut success = false;
                    for attempt in 0..MAX_RETRIES {
                        if handle.is_cancelled() {
                            return Err("cancelled".into());
                        }

                        match super::download::download_chunk(
                            &item.client,
                            &item.chunk_download,
                            &item.chunk,
                            &dest,
                        )
                        .await
                        {
                            Ok(()) => {
                                success = true;
                                break;
                            }
                            Err(e) => {
                                last_err = e.to_string();
                                if attempt < MAX_RETRIES - 1 {
                                    updater(SophonProgress::Warning {
                                        message: format!(
                                            "Chunk {} failed (attempt {}/{}): {last_err}",
                                            item.chunk.chunk_name,
                                            attempt + 1,
                                            MAX_RETRIES
                                        ),
                                    });
                                    let _ = fs::remove_file(&dest);
                                }
                            }
                        }
                    }

                    if !success {
                        let msg = format!(
                            "Failed to download chunk {} after {MAX_RETRIES} attempts: {last_err}",
                            item.chunk.chunk_name
                        );
                        updater(SophonProgress::Error {
                            message: msg.clone(),
                        });
                        return Err(msg);
                    }
                }

                let db = downloaded_bytes.fetch_add(item.chunk.chunk_size, Ordering::Relaxed)
                    + item.chunk.chunk_size;

                adaptive.record_bytes(item.chunk.chunk_size);

                {
                    let mut lu = last_update.lock().unwrap();
                    if lu.elapsed() >= std::time::Duration::from_millis(PROGRESS_UPDATE_INTERVAL_MS)
                    {
                        updater(SophonProgress::Downloading {
                            downloaded_bytes: db,
                            total_bytes: total_compressed,
                        });
                        *lu = Instant::now();
                    }
                }

                let ready: Vec<(usize, usize)> = match chunk_to_files.remove(&item.chunk.chunk_name)
                {
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
                    let _ = assemble_tx.send(entry).await;
                }

                Ok(())
            }
        })
        .buffer_unordered(ADAPTIVE_MAX_CONCURRENCY)
        .collect()
        .await;

    drop(assemble_tx);
    cancel_token.cancel();

    let cancelled = results
        .iter()
        .any(|r| r.as_ref().err().map(|e| e == "cancelled").unwrap_or(false));
    if cancelled {
        let cd = Arc::clone(&chunks_dir);
        let _ = tokio::task::spawn_blocking(move || {
            let _ = fs::remove_dir_all(&*cd);
        })
        .await;
        let _ = assembly_task.await;
        updater(SophonProgress::Finished);
        return Ok(());
    }

    results.into_iter().find(|r| r.is_err()).transpose()?;

    assembly_task
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    {
        let _ = cache::save_verification_cache(game_dir, &verify_cache);
    }

    if !deleted_files.is_empty() {
        let gd = game_dir.to_path_buf();
        let df = deleted_files.clone();
        tokio::task::spawn_blocking(move || {
            for rel in &df {
                let _ = fs::remove_file(gd.join(rel));
            }
        })
        .await
        .map_err(|e| e.to_string())?;
    }

    let gd = game_dir.to_path_buf();
    let tag_str = tag.to_owned();
    let is_pre = is_preinstall;
    tokio::task::spawn_blocking(move || {
        if is_pre {
            fs::write(gd.join(format!(".sophon_preinstall_{tag_str}")), &tag_str)
        } else {
            write_installed_tag(&gd, &tag_str)
        }
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    Ok(())
}

pub async fn apply_preinstall(game_dir: &Path, preinstall_tag: &str) -> Result<(), String> {
    let marker = game_dir.join(format!(".sophon_preinstall_{preinstall_tag}"));
    if !marker.exists() {
        return Err(format!("Preinstall marker for {preinstall_tag} not found"));
    }
    let gd = game_dir.to_path_buf();
    let tag = preinstall_tag.to_owned();
    tokio::task::spawn_blocking(move || {
        write_installed_tag(&gd, &tag).map_err(|e| e.to_string())?;
        fs::remove_file(gd.join(format!(".sophon_preinstall_{tag}"))).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
