use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufWriter, Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use dashmap::DashMap;
use std::sync::Mutex;

use futures_util::StreamExt;
use futures_util::future::try_join_all;
use md5::{Digest as _, Md5};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tauri_plugin_log::log;
use tokio::io::AsyncWriteExt;

use super::api::{
    fetch_build, fetch_front_door, fetch_patch_build, fetch_patch_manifest, is_known_vo_locale,
    vo_lang_matches,
};
use super::error::{SophonError, SophonResult};
use super::handle::DownloadHandle;
use super::read_installed_tag;
use crate::commands::sophon_downloader::api_scrape::{
    DownloadInfo, SophonManifestMeta, SophonPatchManifestMeta,
};

const HDIFF_MAGIC: &[u8; 5] = b"HDIFF";
const PREINSTALL_STATE_FILE_EXT: &str = ".json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatchMethod {
    CopyOver,
    Patch,
    DownloadOver,
    Remove,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchAssetInfo {
    pub target_file_path: String,
    pub target_file_size: u64,
    pub target_file_hash: String,
    pub patch_method: PatchMethod,
    pub patch_name: String,
    pub patch_hash: String,
    pub patch_offset: u64,
    pub patch_size: u64,
    pub patch_chunk_length: u64,
    pub original_file_path: Option<String>,
    pub original_file_hash: Option<String>,
    pub original_file_size: Option<u64>,
    pub matching_field: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchChunkInfo {
    pub patch_name: String,
    pub patch_size: u64,
    pub patch_md5: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreinstallState {
    pub tag: String,
    pub game_id: String,
    pub vo_lang: String,
    pub installed_tag: String,
    pub patch_assets: Vec<PatchAssetInfo>,
    pub deleted_files: Vec<String>,
    pub downloaded_chunks: HashSet<String>,
    pub diff_download: DownloadInfo,
    pub main_chunk_download: DownloadInfo,
    pub main_manifest_ids: Vec<(String, String)>,
}

impl PreinstallState {
    pub fn state_file_path(game_dir: &Path, tag: &str) -> PathBuf {
        game_dir.join(format!(
            ".sophon_preinstall_{tag}{PREINSTALL_STATE_FILE_EXT}"
        ))
    }

    pub fn marker_file_path(game_dir: &Path, tag: &str) -> PathBuf {
        game_dir.join(format!(".sophon_preinstall_{tag}"))
    }
}

pub fn save_preinstall_state(game_dir: &Path, state: &PreinstallState) -> SophonResult<()> {
    let path = PreinstallState::state_file_path(game_dir, &state.tag);
    let tmp_path = path.with_extension("json.tmp");
    {
        let f = fs::File::create(&tmp_path)?;
        let mut writer = BufWriter::new(f);
        serde_json::to_writer(&mut writer, state)
            .map_err(|e| SophonError::PreinstallStateInvalid(e.to_string()))?;
        writer.flush()?;
    }
    fs::rename(&tmp_path, &path)?;
    Ok(())
}

pub fn load_preinstall_state(game_dir: &Path, tag: &str) -> SophonResult<PreinstallState> {
    let path = PreinstallState::state_file_path(game_dir, tag);
    let content = fs::read_to_string(&path).map_err(|e| {
        SophonError::PreinstallStateInvalid(format!("Failed to read preinstall state: {e}"))
    })?;
    serde_json::from_str(&content).map_err(|e| {
        SophonError::PreinstallStateInvalid(format!("Failed to parse preinstall state: {e}"))
    })
}

pub fn delete_preinstall_state(game_dir: &Path, tag: &str) {
    let _ = fs::remove_file(PreinstallState::state_file_path(game_dir, tag));
    let _ = fs::remove_file(PreinstallState::marker_file_path(game_dir, tag));
}

pub struct PreinstallPlan {
    pub patch_assets: Vec<PatchAssetInfo>,
    pub deleted_files: Vec<String>,
    pub unique_chunks: Vec<PatchChunkInfo>,
    pub diff_download: DownloadInfo,
    pub main_chunk_download: DownloadInfo,
    pub tag: String,
    pub main_manifest_ids: Vec<(String, String)>,
}

pub async fn build_preinstall_plan(
    client: &Client,
    game_id: &str,
    vo_lang: &str,
    game_dir: &Path,
) -> SophonResult<PreinstallPlan> {
    let (_, pre_branch) = fetch_front_door(client, game_id).await?;
    let pre_branch = pre_branch.ok_or(SophonError::NoPreinstallAvailable)?;

    let installed_tag = read_installed_tag(game_dir).ok_or(SophonError::NoInstalledVersion)?;

    let patch_build = fetch_patch_build(client, &pre_branch).await?;
    let tag = patch_build.tag.clone();

    let main_build = fetch_build(client, &pre_branch, None).await?;

    let qualifying_patch: Vec<&SophonPatchManifestMeta> = patch_build
        .manifests
        .iter()
        .filter(|m| {
            m.matching_field == "game"
                || vo_lang_matches(&m.matching_field, vo_lang)
                || !is_known_vo_locale(&m.matching_field)
        })
        .collect();

    if qualifying_patch.is_empty() {
        return Err(SophonError::NoGameManifest);
    }

    let main_by_field: HashMap<&str, &SophonManifestMeta> = main_build
        .manifests
        .iter()
        .filter(|m| {
            m.matching_field == "game"
                || vo_lang_matches(&m.matching_field, vo_lang)
                || !is_known_vo_locale(&m.matching_field)
        })
        .map(|m| (m.matching_field.as_str(), m))
        .collect();

    let mut main_manifest_ids: Vec<(String, String)> = Vec::new();
    let mut main_chunk_download: Option<DownloadInfo> = None;

    for (_field, meta) in &main_by_field {
        main_manifest_ids.push((meta.matching_field.clone(), meta.manifest.id.clone()));
        if main_chunk_download.is_none() && meta.matching_field == "game" {
            main_chunk_download = Some(meta.chunk_download.clone());
        }
    }
    let main_chunk_download = main_chunk_download
        .or_else(|| {
            main_by_field
                .values()
                .next()
                .map(|m| m.chunk_download.clone())
        })
        .ok_or(SophonError::NoGameManifest)?;

    let mut all_patch_assets: Vec<PatchAssetInfo> = Vec::new();
    let mut all_deleted_files: Vec<String> = Vec::new();
    let mut seen_chunk_names: HashSet<String> = HashSet::new();
    let mut seen_patch_targets: HashSet<String> = HashSet::new();
    let mut unique_chunks: Vec<PatchChunkInfo> = Vec::new();
    let mut diff_download: Option<DownloadInfo> = None;

    let fetch_futures: Vec<_> = qualifying_patch
        .iter()
        .map(|meta| fetch_patch_manifest(client, meta))
        .collect();
    let patch_results = try_join_all(fetch_futures).await?;

    for result in patch_results {
        let patch_manifest = result.patch_manifest;
        let matching_field = result.matching_field;

        if diff_download.is_none() {
            diff_download = Some(result.diff_download.clone());
        }

        for asset_prop in &patch_manifest.patch_assets {
            let patch_info = asset_prop
                .asset_infos
                .iter()
                .find(|info| info.version_tag.eq_ignore_ascii_case(&installed_tag));

            let has_main_entry = main_by_field.contains_key(matching_field.as_str());

            match patch_info {
                Some(info) => {
                    seen_patch_targets.insert(asset_prop.asset_name.clone());
                    let chunk = match &info.chunk {
                        Some(c) => c,
                        None => {
                            if has_main_entry {
                                all_patch_assets.push(PatchAssetInfo {
                                    target_file_path: asset_prop.asset_name.clone(),
                                    target_file_size: asset_prop.asset_size as u64,
                                    target_file_hash: asset_prop.asset_hash_md5.clone(),
                                    patch_method: PatchMethod::DownloadOver,
                                    patch_name: String::new(),
                                    patch_hash: String::new(),
                                    patch_offset: 0,
                                    patch_size: 0,
                                    patch_chunk_length: 0,
                                    original_file_path: None,
                                    original_file_hash: None,
                                    original_file_size: None,
                                    matching_field: matching_field.clone(),
                                });
                            } else {
                                log::warn!(
                                    "Patch info exists but chunk is None for asset {} (matching_field={}), and no main manifest",
                                    asset_prop.asset_name,
                                    matching_field
                                );
                            }
                            continue;
                        }
                    };

                    let (method, original_file_path, original_file_hash, original_file_size) =
                        if chunk.original_file_name.is_empty() {
                            (PatchMethod::CopyOver, None, None, None)
                        } else {
                            (
                                PatchMethod::Patch,
                                Some(chunk.original_file_name.clone()),
                                Some(chunk.original_file_md5.clone()),
                                Some(chunk.original_file_length as u64),
                            )
                        };

                    if seen_chunk_names.insert(chunk.patch_name.clone()) {
                        unique_chunks.push(PatchChunkInfo {
                            patch_name: chunk.patch_name.clone(),
                            patch_size: chunk.patch_size as u64,
                            patch_md5: chunk.patch_md5.clone(),
                        });
                    }

                    all_patch_assets.push(PatchAssetInfo {
                        target_file_path: asset_prop.asset_name.clone(),
                        target_file_size: asset_prop.asset_size as u64,
                        target_file_hash: asset_prop.asset_hash_md5.clone(),
                        patch_method: method,
                        patch_name: chunk.patch_name.clone(),
                        patch_hash: chunk.patch_md5.clone(),
                        patch_offset: chunk.patch_offset as u64,
                        patch_size: chunk.patch_size as u64,
                        patch_chunk_length: chunk.patch_length as u64,
                        original_file_path,
                        original_file_hash,
                        original_file_size,
                        matching_field: matching_field.clone(),
                    });
                }
                None if has_main_entry => {
                    seen_patch_targets.insert(asset_prop.asset_name.clone());
                    all_patch_assets.push(PatchAssetInfo {
                        target_file_path: asset_prop.asset_name.clone(),
                        target_file_size: asset_prop.asset_size as u64,
                        target_file_hash: asset_prop.asset_hash_md5.clone(),
                        patch_method: PatchMethod::DownloadOver,
                        patch_name: String::new(),
                        patch_hash: String::new(),
                        patch_offset: 0,
                        patch_size: 0,
                        patch_chunk_length: 0,
                        original_file_path: None,
                        original_file_hash: None,
                        original_file_size: None,
                        matching_field: matching_field.clone(),
                    });
                }
                None => {
                    log::warn!(
                        "No patch info for asset {} (matching_field={}) and no main manifest, skipping",
                        asset_prop.asset_name,
                        matching_field
                    );
                }
            }
        }

        for unused_prop in &patch_manifest.unused_assets {
            if !unused_prop.version_tag.eq_ignore_ascii_case(&installed_tag) {
                continue;
            }
            for info in &unused_prop.asset_infos {
                for file in &info.assets {
                    if !seen_patch_targets.contains(file.file_name.as_str()) {
                        all_deleted_files.push(file.file_name.clone());
                    }
                }
            }
        }
    }

    let diff_download = diff_download.ok_or(SophonError::NoGameManifest)?;

    Ok(PreinstallPlan {
        patch_assets: all_patch_assets,
        deleted_files: all_deleted_files,
        unique_chunks,
        diff_download,
        main_chunk_download,
        tag,
        main_manifest_ids,
    })
}

fn patching_chunk_dir(game_dir: &Path) -> PathBuf {
    game_dir.join("patching").join("chunk")
}

type ProgressUpdater = Arc<dyn Fn(SophonProgress) + Send + Sync>;

pub async fn preinstall_download(
    client: &Client,
    plan: &PreinstallPlan,
    game_dir: &Path,
    game_id: &str,
    vo_lang: &str,
    handle: DownloadHandle,
    updater: ProgressUpdater,
    state_saver: Arc<dyn Fn(&HashMap<String, u64>) + Send + Sync>,
    resume_chunks: HashMap<String, u64>,
) -> SophonResult<PreinstallState> {
    let chunks_dir = patching_chunk_dir(game_dir);
    {
        let cd = chunks_dir.clone();
        tokio::task::spawn_blocking(move || fs::create_dir_all(&cd)).await??;
    }

    let installed_tag = read_installed_tag(game_dir).ok_or(SophonError::NoInstalledVersion)?;

    let total_bytes: u64 = plan.unique_chunks.iter().map(|c| c.patch_size).sum();
    let downloaded_bytes = Arc::new(AtomicU64::new(0));
    let resume_offset: u64 = {
        let existing: u64 = plan
            .unique_chunks
            .iter()
            .filter(|c| resume_chunks.contains_key(&c.patch_name))
            .map(|c| c.patch_size)
            .sum();
        existing
    };

    let already_downloaded: HashSet<String> = resume_chunks.keys().cloned().collect();
    let downloaded_chunks: Arc<Mutex<HashSet<String>>> =
        Arc::new(Mutex::new(already_downloaded.clone()));
    let chunk_bytes_map: Arc<DashMap<String, u64>> = Arc::new(DashMap::new());
    for (k, v) in resume_chunks {
        chunk_bytes_map.insert(k, v);
    }

    updater(SophonProgress::Downloading {
        downloaded_bytes: resume_offset,
        total_bytes,
        speed_bps: 0.0,
        eta_seconds: 0.0,
    });

    let start = Instant::now();
    let last_update = Arc::new(std::sync::Mutex::new(Instant::now()));
    let chunks_since_save = Arc::new(AtomicUsize::new(0usize));
    let max_concurrency = super::ADAPTIVE_MAX_CONCURRENCY;

    let chunk_infos: Vec<PatchChunkInfo> = plan.unique_chunks.clone();
    let results: Vec<SophonResult<()>> = futures_util::stream::iter(chunk_infos)
        .map(|chunk_info| {
            let client = client.clone();
            let diff_download = plan.diff_download.clone();
            let chunks_dir = chunks_dir.clone();
            let handle = handle.clone();
            let updater = Arc::clone(&updater);
            let downloaded_bytes = Arc::clone(&downloaded_bytes);
            let downloaded_chunks = Arc::clone(&downloaded_chunks);
            let chunk_bytes_map = Arc::clone(&chunk_bytes_map);
            let state_saver = Arc::clone(&state_saver);
            let last_update = Arc::clone(&last_update);
            let chunks_since_save = Arc::clone(&chunks_since_save);
            let already_downloaded_chunk = already_downloaded.contains(&chunk_info.patch_name);

            async move {
                if handle.is_cancelled() {
                    return Err(SophonError::Cancelled);
                }

                handle
                    .wait_if_paused(
                        &*updater,
                        downloaded_bytes.load(Ordering::Relaxed) + resume_offset,
                        total_bytes,
                    )
                    .await?;

                let chunk_path = chunks_dir.join(&chunk_info.patch_name);

                let needs_download = if chunk_path.exists()
                    && verify_chunk_md5(&chunk_path, &chunk_info.patch_md5)
                {
                    downloaded_bytes.fetch_add(chunk_info.patch_size, Ordering::Relaxed);
                    if !already_downloaded_chunk {
                        downloaded_chunks
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(chunk_info.patch_name.clone());
                        chunk_bytes_map
                            .insert(chunk_info.patch_name.clone(), chunk_info.patch_size);
                    }
                    false
                } else {
                    true
                };

                if needs_download {
                    download_patch_chunk_with_retries(
                        &client,
                        &diff_download,
                        &chunk_info.patch_name,
                        &chunk_path,
                        4,
                    )
                    .await?;

                    downloaded_bytes.fetch_add(chunk_info.patch_size, Ordering::Relaxed);
                    downloaded_chunks
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(chunk_info.patch_name.clone());
                    chunk_bytes_map.insert(chunk_info.patch_name.clone(), chunk_info.patch_size);
                }

                let db = downloaded_bytes.load(Ordering::Relaxed) + resume_offset;
                {
                    if let Ok(mut lu) = last_update.try_lock()
                        && lu.elapsed()
                            >= std::time::Duration::from_millis(super::PROGRESS_UPDATE_INTERVAL_MS)
                    {
                        let total_elapsed = start.elapsed().as_secs_f64();
                        let speed_bps = if total_elapsed > 0.0 {
                            db as f64 / total_elapsed
                        } else {
                            0.0
                        };
                        let remaining = total_bytes.saturating_sub(db);
                        let eta = if speed_bps > 0.0 {
                            remaining as f64 / speed_bps
                        } else {
                            0.0
                        };

                        updater(SophonProgress::Downloading {
                            downloaded_bytes: db,
                            total_bytes,
                            speed_bps,
                            eta_seconds: eta,
                        });
                        *lu = Instant::now();
                    }
                }

                let save_count = chunks_since_save.fetch_add(1, Ordering::Relaxed) + 1;
                if save_count.is_multiple_of(25) {
                    state_saver(&chunk_bytes_map);
                }

                Ok(())
            }
        })
        .buffer_unordered(max_concurrency)
        .collect()
        .await;

    for result in &results {
        if let Err(e) = result {
            if matches!(e, SophonError::Cancelled) {
                return Err(SophonError::Cancelled);
            }
        }
    }
    results.into_iter().find(|r| r.is_err()).transpose()?;

    // Final save to ensure all downloaded chunks are persisted
    state_saver(&chunk_bytes_map);

    updater(SophonProgress::Downloading {
        downloaded_bytes: total_bytes,
        total_bytes,
        speed_bps: 0.0,
        eta_seconds: 0.0,
    });

    let downloaded_chunks: HashSet<String> = downloaded_chunks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain()
        .collect();
    let state = PreinstallState {
        tag: plan.tag.clone(),
        game_id: game_id.to_string(),
        vo_lang: vo_lang.to_string(),
        installed_tag,
        patch_assets: plan.patch_assets.clone(),
        deleted_files: plan.deleted_files.clone(),
        downloaded_chunks,
        diff_download: plan.diff_download.clone(),
        main_chunk_download: plan.main_chunk_download.clone(),
        main_manifest_ids: plan.main_manifest_ids.clone(),
    };

    save_preinstall_state(game_dir, &state)?;

    {
        let gd = game_dir.to_path_buf();
        let tag_str = plan.tag.clone();
        tokio::task::spawn_blocking(move || {
            fs::write(PreinstallState::marker_file_path(&gd, &tag_str), &tag_str)
        })
        .await??;
    }

    Ok(state)
}

async fn download_patch_chunk_with_retries(
    client: &Client,
    diff_download: &DownloadInfo,
    patch_name: &str,
    dest: &Path,
    max_retries: u32,
) -> SophonResult<()> {
    let url = diff_download.url_for(patch_name);
    let mut last_err = String::new();

    for attempt in 0..max_retries {
        match download_patch_chunk_inner(client, &url, dest).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = e.to_string();
                if attempt < max_retries - 1 {
                    log::warn!(
                        "Patch chunk {} failed (attempt {}/{}): {last_err}",
                        patch_name,
                        attempt + 1,
                        max_retries
                    );
                    let _ = fs::remove_file(dest);
                }
            }
        }
    }

    Err(SophonError::DownloadFailed {
        chunk: patch_name.to_string(),
        attempts: max_retries,
        error: last_err,
    })
}

async fn download_patch_chunk_inner(client: &Client, url: &str, dest: &Path) -> SophonResult<()> {
    let resp = client.get(url).send().await?.error_for_status()?;
    let mut stream = resp.bytes_stream();
    let file = tokio::fs::File::create(dest).await?;
    let mut file = tokio::io::BufWriter::new(file);

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        file.write_all(&bytes).await?;
    }
    file.flush().await?;
    Ok(())
}

fn verify_chunk_md5(path: &Path, expected_md5: &str) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Md5::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return false,
        }
    }
    let actual = hex::encode(hasher.finalize());
    actual == expected_md5
}

pub async fn apply_preinstall(
    client: &Client,
    game_dir: &Path,
    preinstall_tag: &str,
    updater: ProgressUpdater,
) -> SophonResult<()> {
    let mut state = load_preinstall_state(game_dir, preinstall_tag)?;

    let current_tag = read_installed_tag(game_dir).ok_or(SophonError::NoInstalledVersion)?;

    if current_tag != state.installed_tag {
        return Err(SophonError::PreinstallStateInvalid(format!(
            "Installed version changed since preinstall (expected {}, got {})",
            state.installed_tag, current_tag
        )));
    }

    let chunks_dir = patching_chunk_dir(game_dir);
    let total_files = state.patch_assets.len() as u64;
    let applied_files = Arc::new(AtomicU64::new(0u64));

    let filter_cache = FilterCache::new(game_dir);
    filter_patch_assets_for_removed_features(&filter_cache, &mut state.patch_assets);

    for asset in &state.patch_assets {
        if asset.patch_method == PatchMethod::Skip {
            applied_files.fetch_add(1, Ordering::Relaxed);
            log::warn!(
                "Skipping patch for removed feature asset: {}",
                asset.target_file_path
            );
            continue;
        }

        let target_path = validate_asset_path(game_dir, &asset.target_file_path)?;
        let already_patched = {
            let tp = target_path.clone();
            let sz = asset.target_file_size;
            let md5 = asset.target_file_hash.clone();
            tokio::task::spawn_blocking(move || is_file_already_patched(&tp, sz, &md5)).await?
        };
        if already_patched {
            applied_files.fetch_add(1, Ordering::Relaxed);
            continue;
        }

        let is_filtered = is_filtered_asset(&filter_cache, asset);

        match asset.patch_method {
            PatchMethod::CopyOver => {
                let gd = game_dir.to_path_buf();
                let cd = chunks_dir.to_path_buf();
                let a = asset.clone();
                let result =
                    tokio::task::spawn_blocking(move || apply_copy_over(&gd, &cd, &a)).await?;
                if let Err(ref e) = result {
                    if is_filtered {
                        log::warn!(
                            "CopyOver failed for filtered asset, skipping: {} ({e})",
                            asset.target_file_path
                        );
                        applied_files.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    log::warn!(
                        "CopyOver failed for {}: {e}, falling back to DownloadOver",
                        asset.target_file_path
                    );
                    apply_download_over(client, game_dir, &state, &asset).await?;
                }
            }
            PatchMethod::Patch => {
                let gd = game_dir.to_path_buf();
                let cd = chunks_dir.to_path_buf();
                let a = asset.clone();
                let fc = filter_cache.clone();
                let result =
                    tokio::task::spawn_blocking(move || apply_hdiff_patch(&gd, &cd, &a, &fc))
                        .await?;
                if let Err(ref e) = result {
                    if is_filtered {
                        log::warn!(
                            "HDiff patch failed for filtered asset, skipping: {} ({e})",
                            asset.target_file_path
                        );
                        applied_files.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    log::warn!(
                        "HDiff patch failed for {}: {e}, falling back to DownloadOver",
                        asset.target_file_path
                    );
                    apply_download_over(client, game_dir, &state, &asset).await?;
                }
            }
            PatchMethod::DownloadOver => {
                apply_download_over(client, game_dir, &state, asset).await?;
            }
            PatchMethod::Remove | PatchMethod::Skip => {}
        }

        let count = applied_files.fetch_add(1, Ordering::Relaxed) + 1;
        updater(SophonProgress::ApplyingPreinstall {
            applied_files: count,
            total_files,
        });
    }

    {
        let gd = game_dir.to_path_buf();
        let df = state.deleted_files.clone();
        tokio::task::spawn_blocking(move || {
            for rel in &df {
                let path = gd.join(rel);
                if path.exists() {
                    let _ = fs::remove_file(&path);
                }
            }
        })
        .await?;
    }

    {
        let gd = game_dir.to_path_buf();
        let tag = state.tag.clone();
        tokio::task::spawn_blocking(move || {
            super::write_installed_tag(&gd, &tag)?;
            let patching_dir = gd.join("patching");
            if patching_dir.exists() {
                let _ = fs::remove_dir_all(&patching_dir);
            }
            delete_preinstall_state(&gd, &tag);
            Ok::<(), SophonError>(())
        })
        .await??;
    }

    Ok(())
}

fn validate_asset_path(game_dir: &Path, asset_path: &str) -> SophonResult<PathBuf> {
    let path = game_dir.join(asset_path);
    if asset_path.starts_with('/') || asset_path.starts_with('\\') || asset_path.contains("..") {
        return Err(SophonError::PathTraversal(path));
    }
    Ok(path)
}

fn is_file_already_patched(path: &Path, expected_size: u64, expected_md5: &str) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if metadata.len() != expected_size {
        return false;
    }
    verify_chunk_md5(path, expected_md5)
}

#[derive(Clone)]
struct FilterCache {
    kdel_tokens: Option<Vec<String>>,
    blacklist_entries: Option<Vec<String>>,
    ignored_lang_patterns: Option<Vec<String>>,
}

impl FilterCache {
    fn new(game_dir: &Path) -> Self {
        let game_dir_str = game_dir.to_string_lossy();

        let kdel_tokens = if game_dir_str.contains("ZenlessZoneZero")
            || game_dir.join("ZenlessZoneZero_Data").exists()
        {
            let kdel_path = game_dir.join("ZenlessZoneZero_Data/Persistent/KDelResource");
            fs::read_to_string(&kdel_path).ok().and_then(|content| {
                let first_line = content.lines().next()?;
                let tokens: Vec<String> = first_line
                    .split(&['|', ';', ',', '$', '#', '@', '+', ' '][..])
                    .map(|token| {
                        token
                            .trim_matches(&['|', ';', ',', '$', '#', '@', '+', ' '][..])
                            .to_string()
                    })
                    .filter(|t| !t.is_empty())
                    .collect();
                if tokens.is_empty() {
                    None
                } else {
                    Some(tokens)
                }
            })
        } else {
            None
        };

        let blacklist_entries = if game_dir.join("StarRail_Data").exists() {
            let blacklist_path = game_dir.join("StarRail_Data/Persistent/DownloadBlacklist.json");
            fs::read_to_string(&blacklist_path)
                .ok()
                .map(|content| {
                    content
                        .lines()
                        .filter_map(|line| extract_blacklist_filename(line))
                        .map(|name| name.to_lowercase())
                        .collect::<Vec<String>>()
                })
                .and_then(|entries| {
                    if entries.is_empty() {
                        None
                    } else {
                        Some(entries)
                    }
                })
        } else {
            None
        };

        let ignored_lang_patterns = if game_dir.join("GenshinImpact_Data").exists()
            || game_dir.join("YuanShen_Data").exists()
        {
            let persistent_dir = find_genshin_persistent_dir(game_dir);
            let installed = read_genshin_installed_langs(&persistent_dir);
            let all_langs: &[&str] = &["Chinese", "English(US)", "Japanese", "Korean"];
            let ignored: Vec<String> = all_langs
                .iter()
                .filter(|lang| !installed.iter().any(|inst| inst == **lang))
                .map(|lang| format!("/{lang}/").to_lowercase())
                .collect();
            Some(ignored)
        } else {
            None
        };

        FilterCache {
            kdel_tokens,
            blacklist_entries,
            ignored_lang_patterns,
        }
    }
}

fn is_filtered_asset(cache: &FilterCache, asset: &PatchAssetInfo) -> bool {
    if let Some(ref tokens) = cache.kdel_tokens {
        for token in tokens {
            if asset.matching_field.eq_ignore_ascii_case(token) {
                return true;
            }
        }
    }

    let asset_lower = asset.target_file_path.to_lowercase();

    if let Some(ref entries) = cache.blacklist_entries {
        for entry in entries {
            if asset_lower.contains(entry.as_str()) {
                return true;
            }
        }
    }

    if let Some(ref patterns) = cache.ignored_lang_patterns {
        for pattern in patterns {
            if asset_lower.contains(pattern.as_str()) {
                return true;
            }
        }
    }

    false
}

fn extract_blacklist_filename(line: &str) -> Option<String> {
    let marker = "\"fileName\":\"";
    let start = line.find(marker)? + marker.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].replace('\\', "/"))
}

fn find_genshin_persistent_dir(game_dir: &Path) -> PathBuf {
    if let Ok(entries) = fs::read_dir(game_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if (name_str == "GenshinImpact_Data" || name_str == "YuanShen_Data")
                && entry.path().is_dir()
            {
                return entry.path().join("Persistent");
            }
        }
    }
    game_dir.join("GenshinImpact_Data/Persistent")
}

fn read_genshin_installed_langs(persistent_dir: &Path) -> Vec<String> {
    if let Ok(entries) = fs::read_dir(persistent_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("audio_lang_") {
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    let langs: Vec<String> = content
                        .lines()
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty())
                        .map(|l| l.to_string())
                        .collect();
                    if !langs.is_empty() {
                        return langs;
                    }
                }
            }
        }
    }
    vec!["English(US)".to_string()]
}

fn filter_patch_assets_for_removed_features(cache: &FilterCache, assets: &mut [PatchAssetInfo]) {
    for asset in assets.iter_mut() {
        if matches!(
            asset.patch_method,
            PatchMethod::DownloadOver | PatchMethod::Patch
        ) && is_filtered_asset(cache, asset)
        {
            asset.patch_method = PatchMethod::Skip;
        }
    }
}

fn apply_copy_over(game_dir: &Path, chunks_dir: &Path, asset: &PatchAssetInfo) -> SophonResult<()> {
    let chunk_path = chunks_dir.join(&asset.patch_name);
    if !chunk_path.exists() {
        return Err(SophonError::PatchChunkNotFound(asset.patch_name.clone()));
    }

    let target_path = validate_asset_path(game_dir, &asset.target_file_path)?;
    let mut chunk_file = fs::File::open(&chunk_path)?;
    chunk_file.seek(SeekFrom::Start(asset.patch_offset))?;

    // Check if this is an HDiff patch by reading just the magic bytes
    let mut magic_buf = [0u8; HDIFF_MAGIC.len()];
    if asset.patch_chunk_length >= HDIFF_MAGIC.len() as u64 {
        chunk_file.read_exact(&mut magic_buf)?;
    }
    if &magic_buf == HDIFF_MAGIC.as_ref() {
        // Need full data in memory for HDiff patching
        let mut data = vec![0u8; asset.patch_chunk_length as usize];
        data[..HDIFF_MAGIC.len()].copy_from_slice(HDIFF_MAGIC.as_ref());
        chunk_file.read_exact(&mut data[HDIFF_MAGIC.len()..])?;
        return apply_hdiff_patch_with_empty_original(game_dir, &data, asset);
    }

    // Stream copy: read from chunk file + write to target without loading all into
    // memory
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temp_path = target_path.with_extension("temp");
    {
        let file = fs::File::create(&temp_path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(&magic_buf)?;
        if asset.patch_chunk_length > magic_buf.len() as u64 {
            let remaining = asset.patch_chunk_length - magic_buf.len() as u64;
            let mut limited = (&mut chunk_file).take(remaining);
            std::io::copy(&mut limited, &mut writer)?;
        }
        writer.flush()?;
    }
    if target_path.exists() {
        let _ = fs::remove_file(&target_path);
    }
    fs::rename(&temp_path, &target_path)?;

    Ok(())
}

fn apply_hdiff_patch(
    game_dir: &Path,
    chunks_dir: &Path,
    asset: &PatchAssetInfo,
    cache: &FilterCache,
) -> SophonResult<()> {
    let original_path = match &asset.original_file_path {
        Some(p) => validate_asset_path(game_dir, p)?,
        None => validate_asset_path(game_dir, &asset.target_file_path)?,
    };

    if !original_path.exists() {
        if is_filtered_asset(cache, asset) {
            log::warn!(
                "Original file missing for filtered asset, skipping: {}",
                asset.target_file_path
            );
            return Ok(());
        }
        return Err(SophonError::OriginalFileMissing(
            original_path.to_string_lossy().to_string(),
        ));
    }

    if let Some(ref expected_size) = asset.original_file_size {
        if original_path.exists() {
            let actual_size = fs::metadata(&original_path).map(|m| m.len()).unwrap_or(0);
            if actual_size != *expected_size {
                if is_filtered_asset(cache, asset) {
                    log::warn!(
                        "Original file size mismatch for filtered asset, skipping: {}",
                        asset.target_file_path
                    );
                    return Ok(());
                }
                return Err(SophonError::OriginalFileMissing(format!(
                    "Size mismatch for {}: expected {}, got {}",
                    original_path.display(),
                    expected_size,
                    actual_size
                )));
            }
        }
    }
    if let Some(ref expected_md5) = asset.original_file_hash {
        if original_path.exists() && !expected_md5.is_empty() {
            if !verify_chunk_md5(&original_path, expected_md5) {
                if is_filtered_asset(cache, asset) {
                    log::warn!(
                        "Original file MD5 mismatch for filtered asset, skipping: {}",
                        asset.target_file_path
                    );
                    return Ok(());
                }
                return Err(SophonError::OriginalFileMissing(format!(
                    "MD5 mismatch for {}",
                    original_path.display()
                )));
            }
        }
    }

    let chunk_path = chunks_dir.join(&asset.patch_name);
    if !chunk_path.exists() {
        return Err(SophonError::PatchChunkNotFound(asset.patch_name.clone()));
    }

    let diff_temp = game_dir.join(format!("patching/{}.diff", asset.patch_name));
    {
        let mut chunk_file = fs::File::open(&chunk_path)?;
        chunk_file.seek(SeekFrom::Start(asset.patch_offset))?;

        if let Some(parent) = diff_temp.parent() {
            fs::create_dir_all(parent)?;
        }
        let diff_file = fs::File::create(&diff_temp)?;
        let mut writer = std::io::BufWriter::new(diff_file);
        let mut limited = (&mut chunk_file).take(asset.patch_chunk_length);
        std::io::copy(&mut limited, &mut writer)?;
        writer.flush()?;
    }

    let target_path = game_dir.join(&asset.target_file_path);
    let temp_output = target_path.with_extension("temp");

    if let Some(parent) = temp_output.parent() {
        fs::create_dir_all(parent)?;
    }

    let op = original_path.to_string_lossy().to_string();
    let dp = diff_temp.to_string_lossy().to_string();
    let tp = temp_output.to_string_lossy().to_string();

    let patch_result = std::thread::spawn(move || {
        let mut hdiff = super::hdiffpatch::HDiff::new(op, dp, tp);
        hdiff.apply()
    })
    .join();

    let _ = fs::remove_file(&diff_temp);

    match patch_result {
        Ok(true) => {
            if target_path.exists() {
                let _ = fs::remove_file(&target_path);
            }
            fs::rename(&temp_output, &target_path)?;
            Ok(())
        }
        Ok(false) => {
            let _ = fs::remove_file(&temp_output);
            Err(SophonError::HDiffPatchFailed {
                file: asset.target_file_path.clone(),
                error: "HDiff apply returned false".to_string(),
            })
        }
        Err(_) => {
            let _ = fs::remove_file(&temp_output);
            Err(SophonError::HDiffPatchFailed {
                file: asset.target_file_path.clone(),
                error: "HDiff thread panicked".to_string(),
            })
        }
    }
}

fn apply_hdiff_patch_with_empty_original(
    game_dir: &Path,
    diff_data: &[u8],
    asset: &PatchAssetInfo,
) -> SophonResult<()> {
    let target_path = validate_asset_path(game_dir, &asset.target_file_path)?;

    let empty_original_path = game_dir.join(format!("patching/{}.diff_ref", asset.patch_name));
    {
        if let Some(parent) = empty_original_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::File::create(&empty_original_path)?;
    }

    let diff_temp = game_dir.join(format!("patching/{}.hdiff", asset.patch_name));
    {
        if let Some(parent) = diff_temp.parent() {
            fs::create_dir_all(parent)?;
        }
        let diff_file = fs::File::create(&diff_temp)?;
        let mut writer = BufWriter::new(diff_file);
        writer.write_all(diff_data)?;
        writer.flush()?;
    }

    let temp_output = target_path.with_extension("temp");
    if let Some(parent) = temp_output.parent() {
        fs::create_dir_all(parent)?;
    }

    let op = empty_original_path.to_string_lossy().to_string();
    let dp = diff_temp.to_string_lossy().to_string();
    let tp = temp_output.to_string_lossy().to_string();

    let patch_result = std::thread::spawn(move || {
        let mut hdiff = super::hdiffpatch::HDiff::new(op, dp, tp);
        hdiff.apply()
    })
    .join();

    let _ = fs::remove_file(&empty_original_path);
    let _ = fs::remove_file(&diff_temp);

    match patch_result {
        Ok(true) => {
            if target_path.exists() {
                let _ = fs::remove_file(&target_path);
            }
            fs::rename(&temp_output, &target_path)?;
            Ok(())
        }
        Ok(false) => {
            let _ = fs::remove_file(&temp_output);
            Err(SophonError::HDiffPatchFailed {
                file: asset.target_file_path.clone(),
                error: "HDiff apply returned false (empty original)".to_string(),
            })
        }
        Err(_) => {
            let _ = fs::remove_file(&temp_output);
            Err(SophonError::HDiffPatchFailed {
                file: asset.target_file_path.clone(),
                error: "HDiff thread panicked (empty original)".to_string(),
            })
        }
    }
}

async fn apply_download_over(
    client: &Client,
    game_dir: &Path,
    state: &PreinstallState,
    asset: &PatchAssetInfo,
) -> SophonResult<()> {
    let target_path = validate_asset_path(game_dir, &asset.target_file_path)?;

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let build = {
        let (_, pre_branch) = fetch_front_door(client, &state.game_id).await?;
        let pre_branch = pre_branch.ok_or(SophonError::NoPreinstallAvailable)?;
        fetch_build(client, &pre_branch, None).await?
    };

    let matching_meta = build
        .manifests
        .iter()
        .find(|m| m.matching_field == asset.matching_field)
        .ok_or_else(|| SophonError::NoVoiceManifest(asset.matching_field.clone()))?;

    let manifest_result = super::api::fetch_manifest(
        client,
        &matching_meta.manifest_download,
        &matching_meta.manifest.id,
    )
    .await?;

    let file_entry = manifest_result
        .manifest
        .assets
        .iter()
        .find(|a| a.asset_name == asset.target_file_path)
        .ok_or_else(|| SophonError::AssemblyFailed {
            file: asset.target_file_path.clone(),
            error: "File not found in main manifest for DownloadOver".to_string(),
        })?;

    let chunks_dir = game_dir.join("chunks");
    {
        let cd = chunks_dir.clone();
        tokio::task::spawn_blocking(move || fs::create_dir_all(&cd)).await??;
    }

    for chunk in &file_entry.asset_chunks {
        let chunk_path = chunks_dir.join(super::assembly::chunk_filename(chunk));
        if !chunk_path.exists() {
            super::download::download_chunk(
                client,
                &matching_meta.chunk_download,
                chunk,
                &chunk_path,
            )
            .await?;
        }
    }

    {
        let gd = game_dir.to_path_buf();
        let file_entry = file_entry.clone();
        let cd = chunks_dir.clone();
        let target = target_path.clone();
        let vc = Arc::new(dashmap::DashMap::new());
        tokio::task::spawn_blocking(move || {
            if target.exists() {
                let _ = fs::remove_file(&target);
            }
            let tmp_dir = gd.join("tmp-patch-downloadover");
            fs::create_dir_all(&tmp_dir)?;
            let result = super::assembly::assemble_file(
                &file_entry,
                &gd,
                &cd,
                &tmp_dir,
                &dashmap::DashMap::new(),
                &vc,
            );
            let _ = fs::remove_dir_all(&tmp_dir);
            result
        })
        .await??;
    }

    Ok(())
}

use crate::commands::sophon_downloader::SophonProgress;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_method_serialization() {
        let method = PatchMethod::CopyOver;
        let json = serde_json::to_string(&method).unwrap();
        assert_eq!(json, "\"copyOver\"");

        let method = PatchMethod::Patch;
        let json = serde_json::to_string(&method).unwrap();
        assert_eq!(json, "\"patch\"");

        let method = PatchMethod::DownloadOver;
        let json = serde_json::to_string(&method).unwrap();
        assert_eq!(json, "\"downloadOver\"");

        let method = PatchMethod::Skip;
        let json = serde_json::to_string(&method).unwrap();
        assert_eq!(json, "\"skip\"");
    }

    #[test]
    fn patch_asset_info_serialization() {
        let info = PatchAssetInfo {
            target_file_path: "GameData/Data.pak".to_string(),
            target_file_size: 1024,
            target_file_hash: "abc123".to_string(),
            patch_method: PatchMethod::Patch,
            patch_name: "chunk_001".to_string(),
            patch_hash: "def456".to_string(),
            patch_offset: 0,
            patch_size: 500,
            patch_chunk_length: 500,
            original_file_path: Some("GameData/Data_old.pak".to_string()),
            original_file_hash: Some("ghi789".to_string()),
            original_file_size: Some(900),
            matching_field: "game".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let deserialized: PatchAssetInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.target_file_path, "GameData/Data.pak");
        assert_eq!(deserialized.patch_method, PatchMethod::Patch);
        assert_eq!(
            deserialized.original_file_path,
            Some("GameData/Data_old.pak".to_string())
        );
    }

    #[test]
    fn preinstall_state_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let state = PreinstallState {
            tag: "5.0.0".to_string(),
            game_id: "hk4e".to_string(),
            vo_lang: "en".to_string(),
            installed_tag: "4.8.0".to_string(),
            patch_assets: vec![PatchAssetInfo {
                target_file_path: "test.pak".to_string(),
                target_file_size: 100,
                target_file_hash: "hash".to_string(),
                patch_method: PatchMethod::CopyOver,
                patch_name: "chunk_0".to_string(),
                patch_hash: "chunkhash".to_string(),
                patch_offset: 0,
                patch_size: 50,
                patch_chunk_length: 50,
                original_file_path: None,
                original_file_hash: None,
                original_file_size: None,
                matching_field: "game".to_string(),
            }],
            deleted_files: vec!["old_file.pak".to_string()],
            downloaded_chunks: HashSet::from(["chunk_0".to_string()]),
            diff_download: make_download_info(),
            main_chunk_download: DownloadInfo {
                encryption: 0,
                password: String::new(),
                compression: crate::commands::sophon_downloader::api_scrape::Compression::None,
                url_prefix: "https://example.com/".to_string(),
                url_suffix: "v2".to_string(),
            },
            main_manifest_ids: vec![("game".to_string(), "manifest_123".to_string())],
        };

        save_preinstall_state(dir.path(), &state).unwrap();
        let loaded = load_preinstall_state(dir.path(), "5.0.0").unwrap();
        assert_eq!(loaded.tag, "5.0.0");
        assert_eq!(loaded.installed_tag, "4.8.0");
        assert_eq!(loaded.patch_assets.len(), 1);
        assert_eq!(loaded.deleted_files.len(), 1);
        assert!(loaded.downloaded_chunks.contains("chunk_0"));
    }

    #[test]
    fn hdiff_magic_detection() {
        assert!(b"HDIFF13".starts_with(HDIFF_MAGIC));
        assert!(!b"NORMAL".starts_with(HDIFF_MAGIC));
    }

    #[test]
    fn preinstall_state_paths() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = PreinstallState::state_file_path(dir.path(), "5.0.0");
        let marker_path = PreinstallState::marker_file_path(dir.path(), "5.0.0");
        assert!(
            state_path
                .to_string_lossy()
                .contains(".sophon_preinstall_5.0.0.json")
        );
        assert!(
            marker_path
                .to_string_lossy()
                .contains(".sophon_preinstall_5.0.0")
        );
        assert!(!marker_path.to_string_lossy().contains(".json"));
    }

    #[test]
    fn verify_chunk_md5_correct() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_chunk");
        let data = b"hello world";
        let md5_hex = hex::encode(md5::Md5::digest(data));
        fs::write(&path, data).unwrap();
        assert!(verify_chunk_md5(&path, &md5_hex));
    }

    #[test]
    fn verify_chunk_md5_wrong() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_chunk");
        fs::write(&path, b"hello world").unwrap();
        assert!(!verify_chunk_md5(&path, "wrong_md5_hash_here"));
    }

    #[test]
    fn verify_chunk_md5_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent");
        assert!(!verify_chunk_md5(&path, "any_hash"));
    }

    #[test]
    fn delete_preinstall_state_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = PreinstallState::state_file_path(dir.path(), "5.0.0");
        let marker_path = PreinstallState::marker_file_path(dir.path(), "5.0.0");
        fs::write(&state_path, "{}").unwrap();
        fs::write(&marker_path, "5.0.0").unwrap();
        assert!(state_path.exists());
        assert!(marker_path.exists());
        delete_preinstall_state(dir.path(), "5.0.0");
        assert!(!state_path.exists());
        assert!(!marker_path.exists());
    }

    #[test]
    fn validate_asset_path_rejects_dotdot() {
        let dir = tempfile::tempdir().unwrap();
        let result = validate_asset_path(dir.path(), "../../../etc/passwd");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, SophonError::PathTraversal(_)));
    }

    #[test]
    fn validate_asset_path_rejects_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let result = validate_asset_path(dir.path(), "/etc/passwd");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SophonError::PathTraversal(_)));
    }

    #[test]
    fn validate_asset_path_rejects_backslash_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let result = validate_asset_path(dir.path(), "\\Windows\\System32");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SophonError::PathTraversal(_)));
    }

    #[test]
    fn validate_asset_path_accepts_normal_relative() {
        let dir = tempfile::tempdir().unwrap();
        let result = validate_asset_path(dir.path(), "GameData/Data.pak");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), dir.path().join("GameData/Data.pak"));
    }

    #[test]
    fn validate_asset_path_accepts_nested_relative() {
        let dir = tempfile::tempdir().unwrap();
        let result = validate_asset_path(dir.path(), "a/b/c/file.pkg");
        assert!(result.is_ok());
    }

    #[test]
    fn is_file_already_patched_size_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        fs::write(&path, b"short").unwrap();
        assert!(!is_file_already_patched(&path, 9999, "any_hash"));
    }

    #[test]
    fn is_file_already_patched_md5_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        let data = b"hello world";
        fs::write(&path, data).unwrap();
        assert!(!is_file_already_patched(
            &path,
            data.len() as u64,
            "wrong_hash"
        ));
    }

    #[test]
    fn is_file_already_patched_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        let data = b"hello world";
        let md5_hex = hex::encode(md5::Md5::digest(data));
        fs::write(&path, data).unwrap();
        assert!(is_file_already_patched(&path, data.len() as u64, &md5_hex));
    }

    #[test]
    fn is_file_already_patched_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.bin");
        assert!(!is_file_already_patched(&path, 100, "any_hash"));
    }

    #[test]
    fn patch_method_remove_serialization() {
        let method = PatchMethod::Remove;
        let json = serde_json::to_string(&method).unwrap();
        assert_eq!(json, "\"remove\"");
        let back: PatchMethod = serde_json::from_str(&json).unwrap();
        assert_eq!(back, PatchMethod::Remove);
    }

    #[test]
    fn patch_method_all_roundtrip() {
        for method in [
            PatchMethod::CopyOver,
            PatchMethod::Patch,
            PatchMethod::DownloadOver,
            PatchMethod::Remove,
            PatchMethod::Skip,
        ] {
            let json = serde_json::to_string(&method).unwrap();
            let back: PatchMethod = serde_json::from_str(&json).unwrap();
            assert_eq!(back, method);
        }
    }

    #[test]
    fn filter_patch_assets_skips_filtered_download_over() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("ZenlessZoneZero_Data/Persistent")).unwrap();
        fs::write(
            dir.path()
                .join("ZenlessZoneZero_Data/Persistent/KDelResource"),
            "cutscenes",
        )
        .unwrap();

        let state = PreinstallState {
            tag: "2.0.0".to_string(),
            game_id: "nap".to_string(),
            vo_lang: "en".to_string(),
            installed_tag: "1.0.0".to_string(),
            patch_assets: vec![
                PatchAssetInfo {
                    target_file_path: "game_data.bin".to_string(),
                    target_file_size: 100,
                    target_file_hash: "h1".to_string(),
                    patch_method: PatchMethod::DownloadOver,
                    patch_name: String::new(),
                    patch_hash: String::new(),
                    patch_offset: 0,
                    patch_size: 0,
                    patch_chunk_length: 0,
                    original_file_path: None,
                    original_file_hash: None,
                    original_file_size: None,
                    matching_field: "cutscenes".to_string(),
                },
                PatchAssetInfo {
                    target_file_path: "core_data.bin".to_string(),
                    target_file_size: 200,
                    target_file_hash: "h2".to_string(),
                    patch_method: PatchMethod::Patch,
                    patch_name: "chunk_0".to_string(),
                    patch_hash: "ph".to_string(),
                    patch_offset: 0,
                    patch_size: 200,
                    patch_chunk_length: 200,
                    original_file_path: Some("core_data_old.bin".to_string()),
                    original_file_hash: Some("oh".to_string()),
                    original_file_size: Some(180),
                    matching_field: "cutscenes".to_string(),
                },
                PatchAssetInfo {
                    target_file_path: "main_game.bin".to_string(),
                    target_file_size: 300,
                    target_file_hash: "h3".to_string(),
                    patch_method: PatchMethod::CopyOver,
                    patch_name: "chunk_1".to_string(),
                    patch_hash: "ph2".to_string(),
                    patch_offset: 0,
                    patch_size: 300,
                    patch_chunk_length: 300,
                    original_file_path: None,
                    original_file_hash: None,
                    original_file_size: None,
                    matching_field: "game".to_string(),
                },
            ],
            deleted_files: vec![],
            downloaded_chunks: HashSet::new(),
            diff_download: make_download_info(),
            main_chunk_download: make_download_info(),
            main_manifest_ids: vec![],
        };

        let cache = FilterCache::new(dir.path());
        let mut assets = state.patch_assets.clone();
        filter_patch_assets_for_removed_features(&cache, &mut assets);
        assert_eq!(assets.len(), 3);
        assert_eq!(assets[0].patch_method, PatchMethod::Skip);
        assert_eq!(assets[1].patch_method, PatchMethod::Skip);
        assert_eq!(assets[2].patch_method, PatchMethod::CopyOver);
    }

    #[test]
    fn filter_patch_assets_no_filter_when_no_kdel() {
        let dir = tempfile::tempdir().unwrap();
        let state = PreinstallState {
            tag: "2.0.0".to_string(),
            game_id: "nap".to_string(),
            vo_lang: "en".to_string(),
            installed_tag: "1.0.0".to_string(),
            patch_assets: vec![PatchAssetInfo {
                target_file_path: "data.bin".to_string(),
                target_file_size: 100,
                target_file_hash: "h".to_string(),
                patch_method: PatchMethod::DownloadOver,
                patch_name: String::new(),
                patch_hash: String::new(),
                patch_offset: 0,
                patch_size: 0,
                patch_chunk_length: 0,
                original_file_path: None,
                original_file_hash: None,
                original_file_size: None,
                matching_field: "cutscenes".to_string(),
            }],
            deleted_files: vec![],
            downloaded_chunks: HashSet::new(),
            diff_download: make_download_info(),
            main_chunk_download: make_download_info(),
            main_manifest_ids: vec![],
        };

        let cache = FilterCache::new(dir.path());
        let mut assets = state.patch_assets.clone();
        filter_patch_assets_for_removed_features(&cache, &mut assets);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].patch_method, PatchMethod::DownloadOver);
    }

    #[test]
    fn apply_copy_over_writes_file() {
        let dir = tempfile::tempdir().unwrap();
        let chunks_dir = dir.path().join("patching/chunk");
        fs::create_dir_all(&chunks_dir).unwrap();

        let data = b"new file content";
        let md5_hex = hex::encode(md5::Md5::digest(data));
        fs::write(chunks_dir.join("patch_0"), data).unwrap();

        let asset = PatchAssetInfo {
            target_file_path: "GameData/output.bin".to_string(),
            target_file_size: data.len() as u64,
            target_file_hash: md5_hex,
            patch_method: PatchMethod::CopyOver,
            patch_name: "patch_0".to_string(),
            patch_hash: String::new(),
            patch_offset: 0,
            patch_size: data.len() as u64,
            patch_chunk_length: data.len() as u64,
            original_file_path: None,
            original_file_hash: None,
            original_file_size: None,
            matching_field: "game".to_string(),
        };

        apply_copy_over(dir.path(), &chunks_dir, &asset).unwrap();

        let written = fs::read(dir.path().join("GameData/output.bin")).unwrap();
        assert_eq!(written, data);
    }

    #[test]
    fn apply_copy_over_with_offset_reads_subrange() {
        let dir = tempfile::tempdir().unwrap();
        let chunks_dir = dir.path().join("patching/chunk");
        fs::create_dir_all(&chunks_dir).unwrap();

        let full_data = b"AAAA_target_content_BBBB";
        fs::write(chunks_dir.join("patch_1"), full_data).unwrap();

        let target_data = &full_data[5..21];
        let md5_hex = hex::encode(md5::Md5::digest(target_data));

        let asset = PatchAssetInfo {
            target_file_path: "GameData/sliced.bin".to_string(),
            target_file_size: target_data.len() as u64,
            target_file_hash: md5_hex,
            patch_method: PatchMethod::CopyOver,
            patch_name: "patch_1".to_string(),
            patch_hash: String::new(),
            patch_offset: 5,
            patch_size: 16,
            patch_chunk_length: 16,
            original_file_path: None,
            original_file_hash: None,
            original_file_size: None,
            matching_field: "game".to_string(),
        };

        apply_copy_over(dir.path(), &chunks_dir, &asset).unwrap();

        let written = fs::read(dir.path().join("GameData/sliced.bin")).unwrap();
        assert_eq!(written, target_data);
    }

    #[test]
    fn apply_copy_over_missing_chunk_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let chunks_dir = dir.path().join("patching/chunk");
        fs::create_dir_all(&chunks_dir).unwrap();

        let asset = PatchAssetInfo {
            target_file_path: "GameData/missing.bin".to_string(),
            target_file_size: 100,
            target_file_hash: "h".to_string(),
            patch_method: PatchMethod::CopyOver,
            patch_name: "nonexistent_chunk".to_string(),
            patch_hash: String::new(),
            patch_offset: 0,
            patch_size: 100,
            patch_chunk_length: 100,
            original_file_path: None,
            original_file_hash: None,
            original_file_size: None,
            matching_field: "game".to_string(),
        };

        let result = apply_copy_over(dir.path(), &chunks_dir, &asset);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SophonError::PatchChunkNotFound(_)
        ));
    }

    #[test]
    fn apply_copy_over_detects_hdiff_magic() {
        let dir = tempfile::tempdir().unwrap();
        let chunks_dir = dir.path().join("patching/chunk");
        fs::create_dir_all(&chunks_dir).unwrap();

        let hdiff_data = b"HDIFF13patchpayload";
        fs::write(chunks_dir.join("patch_hdiff"), hdiff_data).unwrap();

        let asset = PatchAssetInfo {
            target_file_path: "GameData/hdiff.bin".to_string(),
            target_file_size: 0,
            target_file_hash: String::new(),
            patch_method: PatchMethod::CopyOver,
            patch_name: "patch_hdiff".to_string(),
            patch_hash: String::new(),
            patch_offset: 0,
            patch_size: hdiff_data.len() as u64,
            patch_chunk_length: hdiff_data.len() as u64,
            original_file_path: None,
            original_file_hash: None,
            original_file_size: None,
            matching_field: "game".to_string(),
        };

        let result = apply_copy_over(dir.path(), &chunks_dir, &asset);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, SophonError::HDiffPatchFailed { .. }),
            "expected HDiffPatchFailed, got: {err:?}"
        );
    }

    #[test]
    fn load_preinstall_state_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_preinstall_state(dir.path(), "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn load_preinstall_state_corrupted_json() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = PreinstallState::state_file_path(dir.path(), "5.0.0");
        fs::write(&state_path, "not valid json{{{{").unwrap();
        let result = load_preinstall_state(dir.path(), "5.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn save_preinstall_state_atomic_write() {
        let dir = tempfile::tempdir().unwrap();
        let state = PreinstallState {
            tag: "5.0.0".to_string(),
            game_id: "hk4e".to_string(),
            vo_lang: "en".to_string(),
            installed_tag: "4.8.0".to_string(),
            patch_assets: vec![],
            deleted_files: vec![],
            downloaded_chunks: HashSet::new(),
            diff_download: make_download_info(),
            main_chunk_download: make_download_info(),
            main_manifest_ids: vec![],
        };

        save_preinstall_state(dir.path(), &state).unwrap();

        let state_path = PreinstallState::state_file_path(dir.path(), "5.0.0");
        assert!(state_path.exists());
        assert!(!state_path.with_extension("json.tmp").exists());

        let loaded = load_preinstall_state(dir.path(), "5.0.0").unwrap();
        assert_eq!(loaded.tag, "5.0.0");
        assert!(loaded.patch_assets.is_empty());
    }

    #[test]
    fn extract_blacklist_filename_valid() {
        let line = r#"  {"fileName":"Audio/Chinese/abc.pak","fileSize":"1234"}"#;
        let result = extract_blacklist_filename(line);
        assert_eq!(result, Some("Audio/Chinese/abc.pak".to_string()));
    }

    #[test]
    fn extract_blacklist_filename_with_backslashes() {
        let line = r#"  {"fileName":"Audio\Chinese\abc.pak","fileSize":"1234"}"#;
        let result = extract_blacklist_filename(line);
        assert_eq!(result, Some("Audio/Chinese/abc.pak".to_string()));
    }

    #[test]
    fn extract_blacklist_filename_no_match() {
        let line = r#"  {"otherField":"value"}"#;
        let result = extract_blacklist_filename(line);
        assert_eq!(result, None);
    }

    #[test]
    fn extract_blacklist_filename_empty() {
        let result = extract_blacklist_filename("");
        assert_eq!(result, None);
    }

    #[test]
    fn is_filtered_asset_nap_kdel() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("ZenlessZoneZero_Data/Persistent")).unwrap();
        fs::write(
            dir.path()
                .join("ZenlessZoneZero_Data/Persistent/KDelResource"),
            "cutscenes|design",
        )
        .unwrap();

        let cache = FilterCache::new(dir.path());

        let asset = PatchAssetInfo {
            target_file_path: "data.bin".to_string(),
            target_file_size: 100,
            target_file_hash: "h".to_string(),
            patch_method: PatchMethod::Patch,
            patch_name: "c".to_string(),
            patch_hash: "p".to_string(),
            patch_offset: 0,
            patch_size: 100,
            patch_chunk_length: 100,
            original_file_path: None,
            original_file_hash: None,
            original_file_size: None,
            matching_field: "cutscenes".to_string(),
        };
        assert!(is_filtered_asset(&cache, &asset));

        let asset_game = PatchAssetInfo {
            target_file_path: "data.bin".to_string(),
            target_file_size: 100,
            target_file_hash: "h".to_string(),
            patch_method: PatchMethod::Patch,
            patch_name: "c".to_string(),
            patch_hash: "p".to_string(),
            patch_offset: 0,
            patch_size: 100,
            patch_chunk_length: 100,
            original_file_path: None,
            original_file_hash: None,
            original_file_size: None,
            matching_field: "game".to_string(),
        };
        assert!(!is_filtered_asset(&cache, &asset_game));
    }

    #[test]
    fn is_filtered_asset_hkrpg_blacklist() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("StarRail_Data/Persistent")).unwrap();
        fs::write(
            dir.path()
                .join("StarRail_Data/Persistent/DownloadBlacklist.json"),
            r#"{"fileName":"Audio/Korean/vo_kr.pak","fileSize":"1000"}"#,
        )
        .unwrap();

        let cache = FilterCache::new(dir.path());

        let asset = PatchAssetInfo {
            target_file_path: "Audio/Korean/vo_kr.pak".to_string(),
            target_file_size: 100,
            target_file_hash: "h".to_string(),
            patch_method: PatchMethod::DownloadOver,
            patch_name: String::new(),
            patch_hash: String::new(),
            patch_offset: 0,
            patch_size: 0,
            patch_chunk_length: 0,
            original_file_path: None,
            original_file_hash: None,
            original_file_size: None,
            matching_field: "ko-kr".to_string(),
        };
        assert!(is_filtered_asset(&cache, &asset));

        let asset_en = PatchAssetInfo {
            target_file_path: "Audio/English/vo_en.pak".to_string(),
            target_file_size: 100,
            target_file_hash: "h".to_string(),
            patch_method: PatchMethod::DownloadOver,
            patch_name: String::new(),
            patch_hash: String::new(),
            patch_offset: 0,
            patch_size: 0,
            patch_chunk_length: 0,
            original_file_path: None,
            original_file_hash: None,
            original_file_size: None,
            matching_field: "en-us".to_string(),
        };
        assert!(!is_filtered_asset(&cache, &asset_en));
    }

    #[test]
    fn is_filtered_asset_genshin_audio_lang() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("GenshinImpact_Data/Persistent")).unwrap();
        fs::write(
            dir.path()
                .join("GenshinImpact_Data/Persistent/audio_lang_installed"),
            "English(US)\n",
        )
        .unwrap();

        let cache = FilterCache::new(dir.path());

        let asset_en = PatchAssetInfo {
            target_file_path: "Audio/English(US)/vo_en.pak".to_string(),
            target_file_size: 100,
            target_file_hash: "h".to_string(),
            patch_method: PatchMethod::Patch,
            patch_name: "c".to_string(),
            patch_hash: "p".to_string(),
            patch_offset: 0,
            patch_size: 100,
            patch_chunk_length: 100,
            original_file_path: None,
            original_file_hash: None,
            original_file_size: None,
            matching_field: "en-us".to_string(),
        };
        assert!(!is_filtered_asset(&cache, &asset_en));

        let asset_jp = PatchAssetInfo {
            target_file_path: "Audio/Japanese/vo_jp.pak".to_string(),
            target_file_size: 100,
            target_file_hash: "h".to_string(),
            patch_method: PatchMethod::Patch,
            patch_name: "c".to_string(),
            patch_hash: "p".to_string(),
            patch_offset: 0,
            patch_size: 100,
            patch_chunk_length: 100,
            original_file_path: None,
            original_file_hash: None,
            original_file_size: None,
            matching_field: "ja-jp".to_string(),
        };
        assert!(is_filtered_asset(&cache, &asset_jp));
    }

    #[test]
    fn is_filtered_asset_no_game_dir_markers() {
        let dir = tempfile::tempdir().unwrap();
        let cache = FilterCache::new(dir.path());
        let asset = PatchAssetInfo {
            target_file_path: "data.bin".to_string(),
            target_file_size: 100,
            target_file_hash: "h".to_string(),
            patch_method: PatchMethod::Patch,
            patch_name: "c".to_string(),
            patch_hash: "p".to_string(),
            patch_offset: 0,
            patch_size: 100,
            patch_chunk_length: 100,
            original_file_path: None,
            original_file_hash: None,
            original_file_size: None,
            matching_field: "game".to_string(),
        };
        assert!(!is_filtered_asset(&cache, &asset));
    }

    #[test]
    fn patching_chunk_dir_path() {
        let dir = tempfile::tempdir().unwrap();
        let chunks = patching_chunk_dir(dir.path());
        assert!(chunks.to_string_lossy().contains("patching"));
        assert!(chunks.to_string_lossy().contains("chunk"));
    }

    fn make_download_info() -> DownloadInfo {
        DownloadInfo {
            encryption: 0,
            password: String::new(),
            compression: crate::commands::sophon_downloader::api_scrape::Compression::None,
            url_prefix: "https://example.com/".to_string(),
            url_suffix: "v1".to_string(),
        }
    }
}
