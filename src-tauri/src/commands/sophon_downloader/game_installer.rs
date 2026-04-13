use super::SophonProgress;
use super::api_scrape::{
    DownloadInfo, FrontDoorResponse, SophonBuildData, SophonBuildResponse, SophonManifestMeta,
    front_door_game_index,
};
use super::proto_parse::{
    SophonManifestAssetChunk, SophonManifestAssetProperty, SophonManifestProto, decode_manifest,
};
use bytes::BytesMut;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use futures_util::StreamExt;
use md5::{Digest, Md5};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Notify, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

const MAX_RETRIES: u32 = 4;
const ASSEMBLY_CONCURRENCY: usize = 4;
const ASSEMBLY_CHANNEL_SIZE: usize = ASSEMBLY_CONCURRENCY * 4;
const VERSION_FILE_NAME: &str = ".sophon_version";
const VERIFICATION_CACHE_FILE: &str = ".sophon_verify_cache";

const DOWNLOAD_STREAM_BUFFER_SIZE: usize = 256 * 1024;
const FILE_WRITE_BUFFER_SIZE: usize = 1024 * 1024;
const DECOMPRESSION_BUFFER_SIZE: usize = 1024 * 1024;
const MD5_HASH_BUFFER_SIZE: usize = 1024 * 1024;

const PROGRESS_UPDATE_INTERVAL_MS: u64 = 1000;

const ADAPTIVE_MIN_CONCURRENCY: usize = 4;
const ADAPTIVE_MAX_CONCURRENCY: usize = 32;
const ADAPTIVE_INITIAL_CONCURRENCY: usize = 8;
const ADAPTIVE_WINDOW_SECS: u64 = 2;

struct ActiveGuard<'a> {
    adaptive: &'a AdaptiveConcurrency,
}

impl<'a> ActiveGuard<'a> {
    fn new(adaptive: &'a AdaptiveConcurrency) -> Self {
        adaptive.inc_active();
        Self { adaptive }
    }
}

impl<'a> Drop for ActiveGuard<'a> {
    fn drop(&mut self) {
        self.adaptive.dec_active();
    }
}

struct AdaptiveConcurrency {
    target: AtomicUsize,
    active: AtomicUsize,
    total_bytes: AtomicU64,
    window_start: Mutex<Instant>,
    window_start_bytes: AtomicU64,
}

impl AdaptiveConcurrency {
    fn new() -> Self {
        Self {
            target: AtomicUsize::new(ADAPTIVE_INITIAL_CONCURRENCY),
            active: AtomicUsize::new(0),
            total_bytes: AtomicU64::new(0),
            window_start: Mutex::new(Instant::now()),
            window_start_bytes: AtomicU64::new(0),
        }
    }

    fn record_bytes(&self, bytes: u64) {
        self.total_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    fn can_start(&self) -> bool {
        self.active.load(Ordering::Acquire) < self.target.load(Ordering::Acquire)
    }

    fn inc_active(&self) {
        self.active.fetch_add(1, Ordering::AcqRel);
    }

    fn dec_active(&self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }

    fn adjust(&self) -> usize {
        let mut window_start = self.window_start.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(*window_start).as_secs_f64();
        let current = self.target.load(Ordering::Acquire);

        if elapsed < ADAPTIVE_WINDOW_SECS as f64 {
            drop(window_start);
            return current;
        }

        let total = self.total_bytes.load(Ordering::Relaxed);
        let start_bytes = self.window_start_bytes.load(Ordering::Relaxed);
        let bytes_this_window = total.saturating_sub(start_bytes);
        let throughput_bps = bytes_this_window as f64 / elapsed;
        let throughput_mbps = throughput_bps / 1_048_576.0;

        let new_limit = if throughput_mbps > 100.0 {
            (current + 4).min(ADAPTIVE_MAX_CONCURRENCY)
        } else if throughput_mbps > 50.0 {
            (current + 2).min(ADAPTIVE_MAX_CONCURRENCY)
        } else if throughput_mbps > 20.0 {
            current
        } else if throughput_mbps > 10.0 {
            current.saturating_sub(1).max(ADAPTIVE_MIN_CONCURRENCY)
        } else {
            current.saturating_sub(2).max(ADAPTIVE_MIN_CONCURRENCY)
        };

        *window_start = now;
        self.window_start_bytes.store(total, Ordering::Relaxed);
        self.target.store(new_limit, Ordering::Release);
        new_limit
    }
}

const FRONT_DOOR_URL: &str = concat!(
    "https://sg-hyp-api.hoyoverse.com",
    "/hyp/hyp-connect/api/getGameBranches?&launcher_id=VYTpXlbWo8"
);
const SOPHON_BUILD_URL_BASE: &str = concat!(
    "https://sg-public-api.hoyoverse.com",
    "/downloader/sophon_chunk/api/getBuild"
);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VerificationEntry {
    size: u64,
    md5: String,
    mtime_secs: u64,
}

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
            let (cs, ds) = fetch_build_sizes(client, pre, vo_lang)
                .await
                .unwrap_or((0, 0));
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
    vo_lang: &str,
) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
    let build = fetch_build(client, branch, None).await?;
    let game_meta = build.manifests.first().ok_or("no manifests")?;
    let vo_meta = build
        .manifests
        .iter()
        .find(|m| vo_lang_matches(&m.matching_field, vo_lang))
        .ok_or("No VO manifest matching language")?;

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

        // Build map of old files if this manifest existed in old build
        let old_files: HashMap<String, String> = match old_map.get(&new_meta.matching_field) {
            Some(old_meta) => {
                let old_manifest =
                    fetch_manifest(client, &old_meta.manifest_download, &old_meta.manifest.id)
                        .await?;
                old_manifest
                    .assets
                    .into_iter()
                    .filter(|f| !f.is_directory())
                    .map(|f| (f.asset_name, f.asset_hash_md5))
                    .collect()
            }
            None => HashMap::new(),
        };

        // Calculate diff: new or changed files need all chunks
        for file in &new_manifest.assets {
            if file.is_directory() {
                continue;
            }
            let needs_download = match old_files.get(&file.asset_name) {
                Some(old_md5) => old_md5 != &file.asset_hash_md5,
                None => true,
            };
            if needs_download {
                for chunk in &file.asset_chunks {
                    cs += chunk.chunk_size;
                    ds += chunk.chunk_size_decompressed;
                }
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

/// Installer for a single manifest (game or voice-over language pack).
/// Contains the manifest data, HTTP client, and download configuration
/// needed to download and assemble files.
pub struct SophonInstaller {
    client: Client,
    manifest: SophonManifestProto,
    chunk_download: DownloadInfo,
    /// Human-readable label used to name the tmp directory.
    label: String,
    /// The remote build tag this installer was created from.
    #[allow(unused)]
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

struct AssemblyTaskParams {
    file_idx: usize,
    tmp_dir_idx: usize,
    all_files: Arc<Vec<SophonManifestAssetProperty>>,
    all_tmp_dirs: Arc<Vec<PathBuf>>,
    game_dir: PathBuf,
    chunks_dir: Arc<PathBuf>,
    chunk_refcounts: Arc<DashMap<String, usize>>,
    verify_cache: Arc<DashMap<String, VerificationEntry>>,
    assembled_files: Arc<AtomicU64>,
    last_assembly_update: Arc<Mutex<Instant>>,
    total_files: u64,
}

fn spawn_assembly_task(
    params: AssemblyTaskParams,
    updater: impl Fn(SophonProgress) + Send + Sync + 'static,
) -> tokio::task::JoinHandle<Result<(), String>> {
    tokio::task::spawn_blocking(move || {
        run_assembly_task(
            params.file_idx,
            params.tmp_dir_idx,
            params.all_files,
            params.all_tmp_dirs,
            params.game_dir,
            params.chunks_dir,
            params.chunk_refcounts,
            params.verify_cache,
            params.assembled_files,
            params.last_assembly_update,
            updater,
            params.total_files,
        )
    })
}

/// Install all provided installers concurrently into `game_dir`.
///
/// - All installers share `game_dir/chunks` for downloaded chunks.
/// - Downloads and assembly overlap: a file is queued for assembly as soon as
///   all its chunks are on disk.
/// - Multiple files are assembled in parallel up to `ASSEMBLY_CONCURRENCY`.
/// - Respects pause/cancel via `handle`.
/// - After assembly completes, writes `tag` to `game_dir/.sophon_version` (or
///   a preinstall marker file if `is_preinstall` is true).
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

    let all_tmp_dirs: Arc<Vec<PathBuf>> = Arc::new(
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
        Arc::new(load_verification_cache(game_dir));

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
                } else {
                    if let Some(res) = join_set.join_next().await {
                        let _ = res.map_err(|e| e.to_string())?;
                    }
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
            let mut interval = tokio::time::interval(Duration::from_secs(ADAPTIVE_WINDOW_SECS));
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
                // Pause / cancel check before each chunk.
                {
                    let db = downloaded_bytes.load(Ordering::Relaxed);
                    handle
                        .wait_if_paused(&updater, db, total_compressed)
                        .await?;
                }

                let dest = chunks_dir.join(chunk_filename(&item.chunk));

                let needs_download = if dest.exists() {
                    let dest_check = dest.clone();
                    let chunk_size = item.chunk.chunk_size;
                    let expected_md5 = item.chunk.chunk_compressed_hash_md5.clone();
                    let cache = Arc::clone(&verify_cache);
                    !tokio::task::spawn_blocking(move || {
                        check_file_md5_cached(&dest_check, chunk_size, &expected_md5, &cache)
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

                adaptive.record_bytes(item.chunk.chunk_size);

                // Throttle progress updates to 1000ms
                {
                    let mut lu = last_update.lock().unwrap();
                    if lu.elapsed() >= Duration::from_millis(PROGRESS_UPDATE_INTERVAL_MS) {
                        updater(SophonProgress::Downloading {
                            downloaded_bytes: db,
                            total_bytes: total_compressed,
                        });
                        *lu = Instant::now();
                    }
                }

                // Decrement pending counts; queue any newly-ready files.
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

    // Handle cancel: delete all chunks, report Finished (as per spec).
    let cancelled = results
        .iter()
        .any(|r| r.as_ref().err().map(|e| e == "cancelled").unwrap_or(false));
    if cancelled {
        let cd = Arc::clone(&chunks_dir);
        let _ = tokio::task::spawn_blocking(move || {
            let _ = fs::remove_dir_all(&*cd);
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

    {
        let _ = save_verification_cache(game_dir, &verify_cache);
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

        let new_names: HashSet<&str> = new_manifest
            .assets
            .iter()
            .map(|f| f.asset_name.as_str())
            .collect();

        let old_md5_map: HashMap<String, String> =
            match old_by_field.get(new_meta.matching_field.as_str()) {
                Some(old_meta) => {
                    let old_manifest =
                        fetch_manifest(client, &old_meta.manifest_download, &old_meta.manifest.id)
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
                None => HashMap::new(),
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

    let mut buffer = BytesMut::with_capacity(DOWNLOAD_STREAM_BUFFER_SIZE);

    while let Some(chunk_bytes) = stream.next().await {
        let bytes = chunk_bytes?;
        hasher.update(&bytes);
        buffer.extend_from_slice(&bytes);
        if buffer.len() >= DOWNLOAD_STREAM_BUFFER_SIZE {
            file.write_all(&buffer).await?;
            buffer.clear();
        }
        total_len += bytes.len() as u64;
    }

    if !buffer.is_empty() {
        file.write_all(&buffer).await?;
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

fn validate_asset_name(name: &str) -> Result<(), String> {
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

fn run_assembly_task(
    file_idx: usize,
    tmp_dir_idx: usize,
    all_files: Arc<Vec<SophonManifestAssetProperty>>,
    all_tmp_dirs: Arc<Vec<PathBuf>>,
    game_dir: PathBuf,
    chunks_dir: Arc<PathBuf>,
    chunk_refcounts: Arc<DashMap<String, usize>>,
    verify_cache: Arc<DashMap<String, VerificationEntry>>,
    assembled_files: Arc<AtomicU64>,
    last_assembly_update: Arc<Mutex<Instant>>,
    updater: impl Fn(SophonProgress) + Send + Sync + 'static,
    total_files: u64,
) -> Result<(), String> {
    if file_idx >= all_files.len() {
        return Err(format!("file index {} out of bounds", file_idx));
    }
    if tmp_dir_idx >= all_tmp_dirs.len() {
        return Err(format!("tmp_dir index {} out of bounds", tmp_dir_idx));
    }
    let file = &all_files[file_idx];
    let tmp_dir = &all_tmp_dirs[tmp_dir_idx];
    let verify_cache = Arc::clone(&verify_cache);
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

fn assemble_file(
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
        let already_valid = check_file_md5_cached(
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

fn zstd_decompress(bytes: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut decoder = zstd::Decoder::new(bytes)?;
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

fn chunk_filename(chunk: &SophonManifestAssetChunk) -> String {
    format!("{}.zstd", chunk.chunk_name)
}

fn decrement_chunk_refcount(
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

fn file_md5_hex(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Md5::new();
    let mut buf = [0u8; MD5_HASH_BUFFER_SIZE];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VerificationCacheSerializable {
    files: HashMap<String, VerificationEntry>,
}

fn load_verification_cache(game_dir: &Path) -> DashMap<String, VerificationEntry> {
    let cache_path = game_dir.join(VERIFICATION_CACHE_FILE);
    let serializable: VerificationCacheSerializable = match File::open(&cache_path) {
        Ok(f) => serde_json::from_reader(f).unwrap_or(VerificationCacheSerializable {
            files: HashMap::new(),
        }),
        Err(_) => VerificationCacheSerializable {
            files: HashMap::new(),
        },
    };
    let cache = DashMap::new();
    for (k, v) in serializable.files {
        cache.insert(k, v);
    }
    cache
}

fn save_verification_cache(
    game_dir: &Path,
    cache: &DashMap<String, VerificationEntry>,
) -> std::io::Result<()> {
    let cache_path = game_dir.join(VERIFICATION_CACHE_FILE);
    let f = File::create(&cache_path)?;
    let serializable = VerificationCacheSerializable {
        files: cache
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect(),
    };
    serde_json::to_writer(f, &serializable)?;
    Ok(())
}

fn check_file_md5_cached(
    path: &Path,
    expected_size: u64,
    expected_md5: &str,
    cache: &DashMap<String, VerificationEntry>,
) -> std::io::Result<bool> {
    let path_str = path.to_string_lossy().to_string();
    let metadata = path.metadata()?;
    let mtime = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if let Some(entry) = cache.get(&path_str) {
        if entry.size == expected_size && entry.md5 == expected_md5 && entry.mtime_secs == mtime {
            return Ok(true);
        }
    }

    if metadata.len() != expected_size {
        return Ok(false);
    }

    let actual = file_md5_hex(path)?;
    let matches = actual == expected_md5;

    if matches {
        cache.insert(
            path_str,
            VerificationEntry {
                size: expected_size,
                md5: expected_md5.to_string(),
                mtime_secs: mtime,
            },
        );
    }

    Ok(matches)
}
