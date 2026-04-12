use super::SophonProgress;
use super::api_scrape::{
    DownloadInfo, FrontDoorResponse, SophonBuildData, SophonBuildResponse, SophonManifestMeta,
    front_door_game_index,
};
use super::proto_parse::{
    SophonManifestAssetChunk, SophonManifestAssetProperty, SophonManifestProto, decode_manifest,
};
use dashmap::DashMap;
use futures_util::StreamExt;
use md5::{Digest, Md5};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Notify, mpsc};

const MAX_RETRIES: u32 = 4;
const DOWNLOAD_CONCURRENCY: usize = 8;
const ASSEMBLY_CONCURRENCY: usize = 4;
const VERSION_FILE_NAME: &str = ".sophon_version";

const FRONT_DOOR_URL: &str = concat!(
    "https://sg-hyp-api.hoyoverse.com",
    "/hyp/hyp-connect/api/getGameBranches?&launcher_id=VYTpXlbWo8"
);
const SOPHON_BUILD_URL_BASE: &str = concat!(
    "https://sg-public-api.hoyoverse.com",
    "/downloader/sophon_chunk/api/getBuild"
);

/// Shared state used by the pause/resume/cancel Tauri commands.
#[derive(Debug, Clone, PartialEq)]
enum ControlState {
    Running,
    Paused,
    Cancelled,
}

#[derive(Clone)]
pub struct DownloadHandle {
    state: Arc<Mutex<ControlState>>,
    pause_notify: Arc<Notify>,
}

impl DownloadHandle {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(ControlState::Running)),
            pause_notify: Arc::new(Notify::new()),
        }
    }

    pub fn pause(&self) {
        *self.state.lock().unwrap() = ControlState::Paused;
    }

    pub fn resume(&self) {
        *self.state.lock().unwrap() = ControlState::Running;
        self.pause_notify.notify_one();
    }

    pub fn cancel(&self) {
        *self.state.lock().unwrap() = ControlState::Cancelled;
        self.pause_notify.notify_one();
    }

    fn is_cancelled(&self) -> bool {
        *self.state.lock().unwrap() == ControlState::Cancelled
    }

    /// Blocks (async-yields) while paused. Returns `Err` if cancelled.
    async fn wait_if_paused(
        &self,
        updater: &(impl Fn(SophonProgress) + Send + Sync),
        downloaded_bytes: u64,
        total_bytes: u64,
    ) -> Result<(), String> {
        loop {
            let s = self.state.lock().unwrap().clone();
            match s {
                ControlState::Running => return Ok(()),
                ControlState::Cancelled => return Err("cancelled".into()),
                ControlState::Paused => {
                    updater(SophonProgress::Paused {
                        downloaded_bytes,
                        total_bytes,
                    });
                    self.pause_notify.notified().await;
                }
            }
        }
    }
}

fn version_file_path(game_dir: &Path) -> PathBuf {
    game_dir.join(VERSION_FILE_NAME)
}

fn read_installed_tag(game_dir: &Path) -> Option<String> {
    fs::read_to_string(version_file_path(game_dir))
        .ok()
        .map(|s| s.trim().to_owned())
}

/// Public re-export for use from mod.rs.
pub fn read_installed_tag_pub(game_dir: &Path) -> Option<String> {
    read_installed_tag(game_dir)
}

fn write_installed_tag(game_dir: &Path, tag: &str) -> std::io::Result<()> {
    fs::write(version_file_path(game_dir), tag)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    /// Whether a regular update is available (current tag ≠ remote main tag).
    pub update_available: bool,
    /// Whether a preinstall package is currently available from the API.
    pub preinstall_available: bool,
    /// Whether a preinstall has already been fully downloaded and is ready to apply.
    pub preinstall_downloaded: bool,
    /// Currently installed tag, if any.
    pub current_tag: Option<String>,
    /// Remote main-branch tag.
    pub remote_tag: String,
    /// Remote preinstall tag, if a preinstall is live.
    pub preinstall_tag: Option<String>,
    /// Total compressed download size for the update diff (0 if no update).
    pub update_compressed_size: u64,
    /// Total decompressed size for the update diff (0 if no update).
    pub update_decompressed_size: u64,
    /// Total compressed download size for the preinstall (0 if not available).
    pub preinstall_compressed_size: u64,
    /// Total decompressed size for the preinstall (0 if not available).
    pub preinstall_decompressed_size: u64,
}

pub async fn check_update(
    client: &Client,
    game_id: &str,
    vo_lang: &str,
    game_dir: &Path,
) -> Result<UpdateInfo, Box<dyn std::error::Error + Send + Sync>> {
    let (front_door, current_tag) = tokio::join!(fetch_front_door(client, game_id), async {
        read_installed_tag(game_dir)
    },);
    let (branch, pre_download_branch) = front_door?;

    let remote_tag = branch.main.tag.clone();

    let update_available = current_tag
        .as_deref()
        .map(|t| t != remote_tag)
        .unwrap_or(false); // no version file → fresh install, not an update

    // Sizes for the update diff
    let (update_compressed_size, update_decompressed_size) = if update_available {
        if let Some(ref installed) = current_tag {
            match fetch_diff_sizes(client, &branch.main, installed, &remote_tag, vo_lang).await {
                Ok(sizes) => sizes,
                Err(_) => (0, 0),
            }
        } else {
            (0, 0)
        }
    } else {
        (0, 0)
    };

    // Preinstall
    let (
        preinstall_available,
        preinstall_tag,
        preinstall_compressed_size,
        preinstall_decompressed_size,
    ) = match pre_download_branch {
        Some(ref pre) => {
            let tag = pre.tag.clone();
            let (cs, ds) = fetch_build_sizes(client, pre).await.unwrap_or((0, 0));
            (true, Some(tag), cs, ds)
        }
        None => (false, None, 0, 0),
    };

    // Check if preinstall is already downloaded
    let preinstall_downloaded = if let Some(ref ptag) = preinstall_tag {
        game_dir.join(format!(".sophon_preinstall_{ptag}")).exists()
    } else {
        false
    };

    Ok(UpdateInfo {
        update_available,
        preinstall_available,
        preinstall_downloaded,
        current_tag,
        remote_tag,
        preinstall_tag,
        update_compressed_size,
        update_decompressed_size,
        preinstall_compressed_size,
        preinstall_decompressed_size,
    })
}

/// Returns `(compressed_bytes, decompressed_bytes)` for a full build.
async fn fetch_build_sizes(
    client: &Client,
    branch: &super::api_scrape::PackageBranch,
) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
    let build = fetch_build(client, branch, None).await?;
    let game_meta = build.manifests.first().ok_or("no manifests")?;
    let vo_idx = 1usize; // approximate; real index depends on game/lang
    let vo_meta = build.manifests.get(vo_idx).unwrap_or(game_meta);

    let cs =
        parse_size(&game_meta.stats.compressed_size) + parse_size(&vo_meta.stats.compressed_size);
    let ds = parse_size(&game_meta.stats.uncompressed_size)
        + parse_size(&vo_meta.stats.uncompressed_size);
    Ok((cs, ds))
}

/// Returns `(compressed_diff_bytes, decompressed_diff_bytes)` for the set of
/// chunks that differ between `from_tag` and `to_tag`.
async fn fetch_diff_sizes(
    client: &Client,
    branch: &super::api_scrape::PackageBranch,
    from_tag: &str,
    to_tag: &str,
    vo_lang: &str,
) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
    let (old_build, new_build) = tokio::try_join!(
        fetch_build(client, branch, Some(from_tag)),
        fetch_build(client, branch, Some(to_tag)),
    )?;

    let mut cs = 0u64;
    let mut ds = 0u64;

    // Compare manifests pair-wise by matching_field
    let old_map: HashMap<String, &SophonManifestMeta> = old_build
        .manifests
        .iter()
        .map(|m| (m.matching_field.clone(), m))
        .collect();

    for new_meta in &new_build.manifests {
        // Only consider game + selected VO
        if new_meta.matching_field != "game" && !vo_lang_matches(&new_meta.matching_field, vo_lang)
        {
            continue;
        }

        let new_manifest =
            fetch_manifest(client, &new_meta.manifest_download, &new_meta.manifest.id).await?;

        let _old_files: HashMap<String, &SophonManifestAssetProperty> =
            match old_map.get(&new_meta.matching_field) {
                Some(old_meta) => {
                    // We'd need to fetch and cache this too, but for size estimation
                    // we just use the stats delta.
                    let old_cs = parse_size(&old_meta.stats.compressed_size);
                    let new_cs = parse_size(&new_meta.stats.compressed_size);
                    let old_ds = parse_size(&old_meta.stats.uncompressed_size);
                    let new_ds = parse_size(&new_meta.stats.uncompressed_size);
                    // Delta is an approximation; real diff is smaller
                    cs += new_cs.saturating_sub(old_cs);
                    ds += new_ds.saturating_sub(old_ds);
                    continue;
                }
                None => HashMap::new(),
            };
        // If matching_field is entirely new, count everything
        for file in &new_manifest.assets {
            for chunk in &file.asset_chunks {
                cs += chunk.chunk_size;
                ds += chunk.chunk_size_decompressed;
            }
        }
    }

    Ok((cs, ds))
}

fn vo_lang_matches(matching_field: &str, vo_lang: &str) -> bool {
    match vo_lang.to_lowercase().as_str() {
        "cn" => matching_field.contains("zh"),
        "en" => matching_field.contains("en"),
        "jp" => matching_field.contains("ja"),
        "kr" => matching_field.contains("ko"),
        _ => false,
    }
}

fn parse_size(s: &str) -> u64 {
    s.parse().unwrap_or(0)
}

#[allow(unused)]
pub struct SophonInstaller {
    client: Client,
    manifest: SophonManifestProto,
    chunk_download: DownloadInfo,
    /// Human-readable label used to name the tmp directory.
    label: String,
    /// The remote build tag this installer was created from.
    pub tag: String,
}

impl SophonInstaller {
    pub async fn from_manifest_meta(
        client: &Client,
        meta: &SophonManifestMeta,
        tag: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let manifest = fetch_manifest(client, &meta.manifest_download, &meta.manifest.id).await?;
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

/// Install all provided installers concurrently into `game_dir`.
///
/// - All installers share `game_dir/chunks` for downloaded chunks.
/// - Downloads and assembly overlap: a file is queued for assembly as soon as
///   all its chunks are on disk.
/// - Multiple files are assembled in parallel up to `ASSEMBLY_CONCURRENCY`.
/// - Respects pause/cancel via `handle`.
/// - On completion, writes `tag` to `game_dir/.sophon_version`.
/// - `deleted_files` are removed from `game_dir` after assembly completes.
/// - If `is_preinstall` is true, writes a marker file instead of the version
///   file, leaving it for `apply_preinstall` to promote.
pub async fn install(
    installers: Vec<SophonInstaller>,
    game_dir: &Path,
    deleted_files: Vec<String>,
    tag: &str,
    is_preinstall: bool,
    handle: DownloadHandle,
    updater: impl Fn(SophonProgress) + Send + Sync + Clone + 'static,
) -> Result<(), String> {
    let chunks_dir = game_dir.join("chunks");
    {
        let cd = chunks_dir.clone();
        tokio::task::spawn_blocking(move || fs::create_dir_all(&cd))
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
        client: Client,
        chunk_download: DownloadInfo,
        files: Vec<SophonManifestAssetProperty>,
        label: String,
    }

    let installer_data: Vec<InstallerData> = installers
        .into_iter()
        .map(|inst| InstallerData {
            label: inst.label,
            client: inst.client,
            chunk_download: inst.chunk_download,
            files: inst
                .manifest
                .assets
                .into_iter()
                .filter(|a| !a.is_directory())
                .collect(),
        })
        .collect();

    let total_compressed: u64 = installer_data
        .iter()
        .flat_map(|d| d.files.iter())
        .flat_map(|f| f.asset_chunks.iter())
        .map(|c| c.chunk_size)
        .sum();

    let total_files: u64 = installer_data.iter().map(|d| d.files.len() as u64).sum();

    let downloaded_bytes = Arc::new(AtomicU64::new(0));
    let assembled_files = Arc::new(AtomicU64::new(0));

    let chunk_refcounts: Arc<DashMap<String, usize>> = Arc::new(DashMap::new());
    for data in &installer_data {
        for file in &data.files {
            for chunk in &file.asset_chunks {
                chunk_refcounts
                    .entry(chunk.chunk_name.clone())
                    .and_modify(|c| *c += 1)
                    .or_insert(1);
            }
        }
    }

    // Sends (file, tmp_dir) to the assembly pool once all chunks are ready.
    let (assemble_tx, assemble_rx) =
        mpsc::unbounded_channel::<(SophonManifestAssetProperty, PathBuf)>();

    let assembly_task = {
        let chunks_dir = chunks_dir.clone();
        let game_dir = game_dir.to_path_buf();
        let chunk_refcounts = Arc::clone(&chunk_refcounts);
        let assembled_files = Arc::clone(&assembled_files);
        let updater = updater.clone();

        tokio::spawn(async move {
            let mut rx = assemble_rx;
            let mut join_set = tokio::task::JoinSet::new();

            loop {
                // Drain the channel and spawn up to ASSEMBLY_CONCURRENCY tasks.
                while join_set.len() < ASSEMBLY_CONCURRENCY {
                    match rx.try_recv() {
                        Ok((file, tmp_dir)) => {
                            let chunks_dir = chunks_dir.clone();
                            let game_dir = game_dir.clone();
                            let chunk_refcounts = Arc::clone(&chunk_refcounts);
                            let assembled_files = Arc::clone(&assembled_files);
                            let updater = updater.clone();

                            join_set.spawn(tokio::task::spawn_blocking(move || {
                                assemble_file(
                                    &file,
                                    &game_dir,
                                    &chunks_dir,
                                    &tmp_dir,
                                    &chunk_refcounts,
                                )
                                .map_err(|e| {
                                    format!("Failed to assemble {}: {e}", file.asset_name)
                                })?;

                                let count = assembled_files.fetch_add(1, Ordering::Relaxed) + 1;
                                updater(SophonProgress::Assembling {
                                    assembled_files: count,
                                    total_files,
                                });

                                Ok::<(), String>(())
                            }));
                        }
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            // Channel closed — drain remaining tasks and exit.
                            while let Some(res) = join_set.join_next().await {
                                let _ = res.map_err(|e| e.to_string())?;
                            }
                            return Ok::<(), String>(());
                        }
                    }
                }

                if let Some(res) = join_set.join_next().await {
                    let _ = res.map_err(|e| e.to_string())?;
                } else {
                    // Nothing in flight; wait for new work.
                    tokio::task::yield_now().await;
                }
            }
        })
    };

    type PendingCount = Arc<Mutex<usize>>;
    type FileEntry = (SophonManifestAssetProperty, PathBuf, PendingCount);

    struct DownloadItem {
        chunk: SophonManifestAssetChunk,
        client: Client,
        chunk_download: DownloadInfo,
    }

    let chunk_to_files: Arc<DashMap<String, Vec<FileEntry>>> = Arc::new(DashMap::new());
    let mut download_items: Vec<DownloadItem> = Vec::new();
    let mut seen_chunks: HashSet<String> = HashSet::new();

    for data in installer_data {
        let tmp_dir = game_dir.join(format!("tmp-{}", data.label));
        {
            let td = tmp_dir.clone();
            tokio::task::spawn_blocking(move || fs::create_dir_all(&td))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
        }

        for file in data.files {
            let chunk_count = file.asset_chunks.len();
            if chunk_count == 0 {
                let _ = assemble_tx.send((file, tmp_dir.clone()));
                continue;
            }

            let pending = Arc::new(Mutex::new(chunk_count));
            for chunk in &file.asset_chunks {
                chunk_to_files
                    .entry(chunk.chunk_name.clone())
                    .or_default()
                    .push((file.clone(), tmp_dir.clone(), Arc::clone(&pending)));

                if seen_chunks.insert(chunk.chunk_name.clone()) {
                    download_items.push(DownloadItem {
                        chunk: chunk.clone(),
                        client: data.client.clone(),
                        chunk_download: data.chunk_download.clone(),
                    });
                }
            }
        }
    }

    let last_update: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));

    let results: Vec<Result<(), String>> = futures_util::stream::iter(download_items)
        .map(|item| {
            let chunks_dir = chunks_dir.clone();
            let downloaded_bytes = Arc::clone(&downloaded_bytes);
            let chunk_to_files = Arc::clone(&chunk_to_files);
            let assemble_tx = assemble_tx.clone();
            let handle = handle.clone();
            let updater = updater.clone();
            let last_update = Arc::clone(&last_update);

            async move {
                // Pause / cancel check before each chunk.
                {
                    let db = downloaded_bytes.load(Ordering::Relaxed);
                    handle
                        .wait_if_paused(&updater, db, total_compressed)
                        .await?;
                }

                let dest = chunks_dir.join(chunk_filename(&item.chunk));

                // Skip if already valid on disk.
                let dest_check = dest.clone();
                let chunk_size = item.chunk.chunk_size;
                let expected_md5 = item.chunk.chunk_compressed_hash_md5.clone();
                let already_done = tokio::task::spawn_blocking(move || {
                    dest_check.exists() && check_file_md5(&dest_check, chunk_size, &expected_md5)
                })
                .await
                .map_err(|e| e.to_string())?;

                if !already_done {
                    let mut last_err = String::new();
                    let mut success = false;
                    for attempt in 0..MAX_RETRIES {
                        // Check for cancel/pause between retry attempts too.
                        if handle.is_cancelled() {
                            return Err("cancelled".into());
                        }

                        match download_chunk(&item.client, &item.chunk_download, &item.chunk, &dest)
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

                // Update download counter with atomic fetch_add
                let db = downloaded_bytes.fetch_add(item.chunk.chunk_size, Ordering::Relaxed)
                    + item.chunk.chunk_size;

                // Throttle progress updates to 1000ms
                {
                    let mut lu = last_update.lock().unwrap();
                    if lu.elapsed() >= Duration::from_millis(1000) {
                        updater(SophonProgress::Downloading {
                            downloaded_bytes: db,
                            total_bytes: total_compressed,
                        });
                        *lu = Instant::now();
                    }
                }

                // Decrement pending counts; queue any newly-ready files.
                let ready: Vec<(SophonManifestAssetProperty, PathBuf)> =
                    match chunk_to_files.remove(&item.chunk.chunk_name) {
                        Some((_, entries)) => entries
                            .into_iter()
                            .filter_map(|(file, tmp_dir, pending)| {
                                let mut count = pending.lock().unwrap();
                                *count -= 1;
                                if *count == 0 {
                                    Some((file, tmp_dir))
                                } else {
                                    None
                                }
                            })
                            .collect(),
                        None => Vec::new(),
                    };

                for entry in ready {
                    let _ = assemble_tx.send(entry);
                }

                Ok(())
            }
        })
        .buffer_unordered(DOWNLOAD_CONCURRENCY)
        .collect()
        .await;

    drop(assemble_tx);

    // Handle cancel: delete all chunks, report Finished (as per spec).
    let cancelled = results
        .iter()
        .any(|r| r.as_ref().err().map(|e| e == "cancelled").unwrap_or(false));
    if cancelled {
        let cd = chunks_dir.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = fs::remove_dir_all(&cd);
        })
        .await;
        // Wait for assembly to drain before returning.
        let _ = assembly_task.await;
        updater(SophonProgress::Finished);
        return Ok(());
    }

    // Surface first real error.
    results.into_iter().find(|r| r.is_err()).transpose()?;

    // Wait for all assembly tasks.
    assembly_task
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    // Delete files removed in this version.
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

    // Write version marker.
    let gd = game_dir.to_path_buf();
    let tag_str = tag.to_owned();
    let is_pre = is_preinstall;
    tokio::task::spawn_blocking(move || {
        if is_pre {
            // Write a preinstall-ready marker instead of overwriting the live version.
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

/// Promote a previously downloaded preinstall to the live install by renaming
/// the marker file to the version file. Call this after `install(..., is_preinstall=true)`.
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

async fn fetch_front_door(
    client: &Client,
    game_id: &str,
) -> Result<
    (
        super::api_scrape::GameBranch,
        Option<super::api_scrape::PackageBranch>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    let resp: FrontDoorResponse = client.get(FRONT_DOOR_URL).send().await?.json().await?;

    let idx =
        front_door_game_index(game_id).ok_or_else(|| format!("Unknown game_id: {game_id}"))?;

    let branch = resp
        .data
        .game_branches
        .into_iter()
        .nth(idx)
        .ok_or("Front-door branch index out of range")?;

    let pre = branch.pre_download.clone();
    Ok((branch, pre))
}

async fn fetch_build(
    client: &Client,
    branch: &super::api_scrape::PackageBranch,
    tag: Option<&str>,
) -> Result<SophonBuildData, Box<dyn std::error::Error + Send + Sync>> {
    let mut url = format!(
        "{}?branch={}&package_id={}&password={}",
        SOPHON_BUILD_URL_BASE, branch.branch, branch.package_id, branch.password,
    );
    if let Some(t) = tag {
        url.push_str(&format!("&tag={t}"));
    }

    let resp: SophonBuildResponse = client.get(&url).send().await?.json().await?;
    if resp.data.manifests.is_empty() {
        return Err("No manifests returned from the API".into());
    }
    Ok(resp.data)
}

async fn build_installers_from_data(
    client: &Client,
    build: &SophonBuildData,
    vo_lang: &str,
    tag: &str,
) -> Result<Vec<SophonInstaller>, Box<dyn std::error::Error + Send + Sync>> {
    // Determine game_id from context isn't available here; use index 0 for game
    // and find the VO by matching_field.
    let game_meta = build.manifests.first().ok_or("No game manifest")?;

    // Find VO manifest by matching_field name.
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

/// Returns `(installers_for_changed_files, names_of_deleted_files)`.
async fn build_diff_installers(
    client: &Client,
    old_build: &SophonBuildData,
    new_build: &SophonBuildData,
    vo_lang: &str,
    tag: &str,
) -> Result<(Vec<SophonInstaller>, Vec<String>), Box<dyn std::error::Error + Send + Sync>> {
    let old_by_field: HashMap<&str, &SophonManifestMeta> = old_build
        .manifests
        .iter()
        .map(|m| (m.matching_field.as_str(), m))
        .collect();

    let mut installers = Vec::new();
    let mut deleted_files: Vec<String> = Vec::new();

    for new_meta in &new_build.manifests {
        // Only process game + selected VO.
        if new_meta.matching_field != "game" && !vo_lang_matches(&new_meta.matching_field, vo_lang)
        {
            continue;
        }

        let new_manifest =
            fetch_manifest(client, &new_meta.manifest_download, &new_meta.manifest.id).await?;

        // Build a map of old files by name → md5.
        let _old_file_map: HashMap<&str, &str> =
            match old_by_field.get(new_meta.matching_field.as_str()) {
                Some(old_meta) => {
                    // Fetch old manifest to compare file-by-file.
                    let old_manifest =
                        fetch_manifest(client, &old_meta.manifest_download, &old_meta.manifest.id)
                            .await?;

                    // Files present in old but absent in new → deleted.
                    let new_names: HashSet<&str> = new_manifest
                        .assets
                        .iter()
                        .map(|f| f.asset_name.as_str())
                        .collect();

                    // We hold old_manifest here temporarily; collect deleted names.
                    let mut dels: Vec<String> = old_manifest
                        .assets
                        .iter()
                        .filter(|f| !f.is_directory() && !new_names.contains(f.asset_name.as_str()))
                        .map(|f| f.asset_name.clone())
                        .collect();
                    deleted_files.append(&mut dels);

                    // Build old md5 map — but old_manifest is consumed above, so we
                    // need to refetch. To avoid a double fetch we'll do it differently:
                    // collect into a local vec first.
                    HashMap::new() // populated below after second fetch
                }
                None => HashMap::new(), // entirely new category → all files are new
            };

        // Re-fetch old manifest to compare md5s (only if it existed).
        let old_md5_map: HashMap<String, String> =
            if let Some(old_meta) = old_by_field.get(new_meta.matching_field.as_str()) {
                let om = fetch_manifest(client, &old_meta.manifest_download, &old_meta.manifest.id)
                    .await?;
                om.assets
                    .into_iter()
                    .filter(|f| !f.is_directory())
                    .map(|f| (f.asset_name, f.asset_hash_md5))
                    .collect()
            } else {
                HashMap::new()
            };

        // Keep only files that are new or have a changed MD5.
        let diff_files: Vec<SophonManifestAssetProperty> = new_manifest
            .assets
            .into_iter()
            .filter(|f| {
                if f.is_directory() {
                    return false;
                }
                match old_md5_map.get(&f.asset_name) {
                    Some(old_md5) => old_md5 != &f.asset_hash_md5,
                    None => true, // new file
                }
            })
            .collect();

        if diff_files.is_empty() {
            continue;
        }

        // Build a synthetic SophonInstaller containing only changed files.
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

async fn download_chunk(
    client: &Client,
    chunk_download: &DownloadInfo,
    chunk: &SophonManifestAssetChunk,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = chunk_download.url_for(&chunk.chunk_name);
    let resp = client.get(&url).send().await?.error_for_status()?;

    if let Some(len) = resp.content_length() {
        if len != chunk.chunk_size {
            return Err(format!(
                "Content-Length mismatch for {}: expected {}, got {len}",
                chunk.chunk_name, chunk.chunk_size
            )
            .into());
        }
    }

    let mut file = tokio::fs::File::create(dest).await?;
    let mut stream = resp.bytes_stream();
    let mut hasher = Md5::new();
    let mut total_len = 0u64;

    while let Some(chunk_bytes) = stream.next().await {
        let bytes = chunk_bytes?;
        hasher.update(&bytes);
        file.write_all(&bytes).await?;
        total_len += bytes.len() as u64;
    }

    if total_len != chunk.chunk_size {
        return Err(format!(
            "Downloaded size mismatch for {}: expected {}, got {}",
            chunk.chunk_name, chunk.chunk_size, total_len
        )
        .into());
    }

    if !chunk.chunk_compressed_hash_md5.is_empty() {
        let actual = format!("{:x}", hasher.finalize());
        if actual != chunk.chunk_compressed_hash_md5 {
            return Err(format!(
                "Compressed MD5 mismatch for {}: expected {}, got {actual}",
                chunk.chunk_name, chunk.chunk_compressed_hash_md5
            )
            .into());
        }
    }

    Ok(())
}

async fn fetch_manifest(
    client: &Client,
    dl: &DownloadInfo,
    manifest_id: &str,
) -> Result<SophonManifestProto, Box<dyn std::error::Error + Send + Sync>> {
    let url = dl.url_for(manifest_id);
    let bytes = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let raw = if dl.is_compressed() {
        tokio::task::spawn_blocking(move || zstd_decompress(&bytes)).await??
    } else {
        bytes.to_vec()
    };

    decode_manifest(&raw).map_err(|e| e.into())
}

/// Delete all leftover `.tmp` files in `dir` recursively.
fn cleanup_tmp_files(dir: &Path) -> std::io::Result<()> {
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

fn assemble_file(
    file: &SophonManifestAssetProperty,
    game_dir: &Path,
    chunks_dir: &Path,
    temp_dir: &Path,
    chunk_refcounts: &DashMap<String, usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let target_path = game_dir.join(&file.asset_name);
    let tmp_path = temp_dir.join(format!("{}.tmp", md5_hex(file.asset_name.as_bytes())));

    if target_path.exists() && check_file_md5(&target_path, file.asset_size, &file.asset_hash_md5) {
        for chunk in &file.asset_chunks {
            if let Some(mut count) = chunk_refcounts.get_mut(&chunk.chunk_name) {
                *count -= 1;
                if *count == 0 {
                    drop(count);
                    chunk_refcounts.remove(&chunk.chunk_name);
                    let _ = fs::remove_file(chunks_dir.join(chunk_filename(chunk)));
                }
            }
        }
        return Ok(());
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

    let mut total_written: u64 = 0;
    let mut hasher = Md5::new();

    for chunk in &file.asset_chunks {
        let chunk_path = chunks_dir.join(chunk_filename(chunk));
        let decompressed = decompress_chunk(&chunk_path)?;

        hasher.update(&decompressed);

        if !chunk.chunk_decompressed_hash_md5.is_empty() {
            let actual = md5_hex(&decompressed);
            if actual != chunk.chunk_decompressed_hash_md5 {
                return Err(format!(
                    "Decompressed MD5 mismatch for chunk {} in file {}: expected {}, got {actual}",
                    chunk.chunk_name, file.asset_name, chunk.chunk_decompressed_hash_md5
                )
                .into());
            }
        }

        let written = write_all_at(&out_file, &decompressed, chunk.chunk_on_file_offset)?;
        if written != chunk.chunk_size_decompressed {
            return Err(format!(
                "Chunk {} written {} bytes but expected {}",
                chunk.chunk_name, written, chunk.chunk_size_decompressed
            )
            .into());
        }
        total_written += written;

        if let Some(mut count) = chunk_refcounts.get_mut(&chunk.chunk_name) {
            *count -= 1;
            if *count == 0 {
                drop(count);
                chunk_refcounts.remove(&chunk.chunk_name);
                let _ = fs::remove_file(&chunk_path);
            }
        }
    }

    out_file.sync_data()?;
    drop(out_file);

    if total_written != file.asset_size {
        return Err(format!(
            "File {} total written {} != expected {}",
            file.asset_name, total_written, file.asset_size
        )
        .into());
    }

    if !file.asset_hash_md5.is_empty() {
        let actual = format!("{:x}", hasher.finalize());
        if actual != file.asset_hash_md5 {
            return Err(format!(
                "Final file MD5 mismatch for {}: expected {}, got {actual}",
                file.asset_name, file.asset_hash_md5
            )
            .into());
        }
    }

    fs::rename(&tmp_path, &target_path)?;
    Ok(())
}

fn decompress_chunk(path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let f = File::open(path)?;
    let mut decoder = zstd::Decoder::new(f)?;
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

fn zstd_decompress(bytes: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut decoder = zstd::Decoder::new(bytes)?;
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

fn write_all_at(file: &File, data: &[u8], offset: u64) -> std::io::Result<u64> {
    let mut written = 0usize;
    while written < data.len() {
        let n = file.write_at(&data[written..], offset + written as u64)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "write_at returned 0",
            ));
        }
        written += n;
    }
    Ok(written as u64)
}

fn chunk_filename(chunk: &SophonManifestAssetChunk) -> String {
    format!("{}.zstd", chunk.chunk_name)
}

fn md5_hex(data: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn file_md5_hex(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Md5::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn check_file_md5(path: &Path, expected_size: u64, expected_md5: &str) -> bool {
    if expected_md5.is_empty() {
        return false;
    }
    match path.metadata() {
        Ok(m) if m.len() == expected_size => {}
        _ => return false,
    }
    match file_md5_hex(path) {
        Ok(actual) => actual == expected_md5,
        Err(_) => false,
    }
}
