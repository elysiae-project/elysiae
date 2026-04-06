use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use reqwest::Client;
use tokio::sync::mpsc;

use super::api_scrape::{
    DownloadInfo, FrontDoorResponse, SophonBuildResponse, SophonManifestMeta,
    front_door_game_index, vo_manifest_index,
};
use super::proto_parse::{
    SophonManifestAssetChunk, SophonManifestAssetProperty, SophonManifestProto, decode_manifest,
};
use super::SophonProgress;


const MAX_RETRIES: u32 = 4;
const DOWNLOAD_CONCURRENCY: usize = 8;
const FRONT_DOOR_URL: &str = concat!(
    "https://sg-hyp-api.hoyoverse.com",
    "/hyp/hyp-connect/api/getGameBranches?&launcher_id=VYTpXlbWo8"
);
const SOPHON_BUILD_URL_BASE: &str =
    "https://sg-public-api.hoyoverse.com/downloader/sophon_chunk/api/getBuild";

pub struct SophonInstaller {
    client: Client,
    manifest: SophonManifestProto,
    chunk_download: DownloadInfo,
}

impl SophonInstaller {
    pub async fn from_manifest_meta(
        client: &Client,
        meta: &SophonManifestMeta,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let manifest =
            fetch_manifest(client, &meta.manifest_download, &meta.manifest.id).await?;
        Ok(Self {
            client: client.clone(),
            manifest,
            chunk_download: meta.chunk_download.clone(),
        })
    }
}

pub async fn build_installers(
    client: &Client,
    game_id: &str,
    vo_lang: &str,
) -> Result<Vec<SophonInstaller>, Box<dyn std::error::Error + Send + Sync>> {
    let branch_resp: FrontDoorResponse = client
        .get(FRONT_DOOR_URL)
        .send()
        .await?
        .json()
        .await?;

    let branch_idx =
        front_door_game_index(game_id).ok_or_else(|| format!("Unknown game_id: {game_id}"))?;

    let branch = branch_resp
        .data
        .game_branches
        .get(branch_idx)
        .ok_or("Front-door branch index out of range")?;

    let build_url = format!(
        "{}?branch={}&package_id={}&password={}",
        SOPHON_BUILD_URL_BASE,
        branch.main.branch,
        branch.main.package_id,
        branch.main.password,
    );

    let build_resp: SophonBuildResponse = client
        .get(&build_url)
        .send()
        .await?
        .json()
        .await?;

    let manifests = &build_resp.data.manifests;
    if manifests.is_empty() {
        return Err("No manifests returned from the API".into());
    }

    // Get Game and VO manifests
    let vo_idx =
        vo_manifest_index(game_id, vo_lang).ok_or_else(|| format!("Unknown vo_lang: {vo_lang}"))?;
    let vo_meta = manifests
        .get(vo_idx)
        .ok_or("VO manifest index out of range")?;

    let (game_installer, vo_installer) = tokio::try_join!(
        SophonInstaller::from_manifest_meta(client, &manifests[0]),
        SophonInstaller::from_manifest_meta(client, vo_meta),
    )?;

    Ok(vec![game_installer, vo_installer])
}

pub async fn install(
    installers: Vec<SophonInstaller>,
    game_dir: &Path,
    updater: impl Fn(SophonProgress) + Send + Sync + Clone + 'static,
) -> Result<(), String> {
    let chunks_dir = game_dir.join("chunks");
    tokio::task::spawn_blocking({
        let chunks_dir = chunks_dir.clone();
        move || fs::create_dir_all(&chunks_dir)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    struct InstallerData {
        client: Client,
        chunk_download: DownloadInfo,
        files: Vec<SophonManifestAssetProperty>,
        label: String,
    }

    let installer_data: Vec<InstallerData> = installers
        .into_iter()
        .map(|inst| InstallerData {
            label: inst
                .chunk_download
                .url_suffix
                .trim_matches('/')
                .replace('/', "-"),
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

    let total_files: u64 = installer_data
        .iter()
        .map(|d| d.files.len() as u64)
        .sum();

    let downloaded_bytes = Arc::new(Mutex::new(0u64));
    let assembled_files = Arc::new(Mutex::new(0u64));

    let chunk_refcounts: Arc<Mutex<HashMap<String, usize>>> = {
        let mut map = HashMap::new();
        for data in &installer_data {
            for file in &data.files {
                for chunk in &file.asset_chunks {
                    *map.entry(chunk.chunk_name.clone()).or_insert(0) += 1;
                }
            }
        }
        Arc::new(Mutex::new(map))
    };

    let (assemble_tx, mut assemble_rx) =
        mpsc::unbounded_channel::<(SophonManifestAssetProperty, PathBuf)>();

    let assembly_task = {
        let chunks_dir = chunks_dir.clone();
        let game_dir = game_dir.to_path_buf();
        let chunk_refcounts = Arc::clone(&chunk_refcounts);
        let assembled_files = Arc::clone(&assembled_files);
        let updater = updater.clone();

        tokio::spawn(async move {
            while let Some((file, tmp_dir)) = assemble_rx.recv().await {
                let chunks_dir = chunks_dir.clone();
                let game_dir = game_dir.clone();
                let chunk_refcounts = Arc::clone(&chunk_refcounts);
                let assembled_files = Arc::clone(&assembled_files);
                let updater = updater.clone();

                tokio::task::spawn_blocking(move || {
                    assemble_file(
                        &file,
                        &game_dir,
                        &chunks_dir,
                        &tmp_dir,
                        &chunk_refcounts,
                    )
                    .map_err(|e| format!("Failed to assemble {}: {e}", file.asset_name))?;

                    let count = {
                        let mut guard = assembled_files.lock().unwrap();
                        *guard += 1;
                        *guard
                    };
                    updater(SophonProgress::Assembling {
                        assembled_files: count,
                        total_files,
                    });

                    Ok::<(), String>(())
                })
                .await
                .map_err(|e| e.to_string())??;
            }
            Ok::<(), String>(())
        })
    };

    type PendingCount = Arc<Mutex<usize>>;
    type FileEntry = (SophonManifestAssetProperty, PathBuf, PendingCount);

    struct DownloadItem {
        chunk: SophonManifestAssetChunk,
        client: Client,
        chunk_download: DownloadInfo,
    }

    let mut chunk_to_files: HashMap<String, Vec<FileEntry>> = HashMap::new();
    let mut download_items: Vec<DownloadItem> = Vec::new();
    let mut seen_chunks: std::collections::HashSet<String> = std::collections::HashSet::new();

    for data in installer_data {
        let tmp_dir = game_dir.join(format!("tmp-{}", data.label));
        tokio::task::spawn_blocking({
            let tmp_dir = tmp_dir.clone();
            move || fs::create_dir_all(&tmp_dir)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

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

    let chunk_to_files = Arc::new(Mutex::new(chunk_to_files));
    let results: Vec<Result<(), String>> = futures_util::stream::iter(download_items)
        .map(|item| {
            let chunks_dir = chunks_dir.clone();
            let downloaded_bytes = Arc::clone(&downloaded_bytes);
            let chunk_to_files = Arc::clone(&chunk_to_files);
            let assemble_tx = assemble_tx.clone();
            let updater = updater.clone();

            async move {
                let dest = chunks_dir.join(chunk_filename(&item.chunk));

                // Skip if already on disk and valid
                let dest_check = dest.clone();
                let chunk_size = item.chunk.chunk_size;
                let expected_md5 = item.chunk.chunk_compressed_hash_md5.clone();
                let already_done = tokio::task::spawn_blocking(move || {
                    dest_check.exists()
                        && check_file_md5(&dest_check, chunk_size, &expected_md5)
                })
                .await
                .map_err(|e| e.to_string())?;

                if !already_done {
                    let mut last_err = String::new();
                    let mut success = false;
                    for attempt in 0..MAX_RETRIES {
                        match download_chunk(
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
                        updater(SophonProgress::Error { message: msg.clone() });
                        return Err(msg);
                    }
                }
                {
                    let mut guard = downloaded_bytes.lock().unwrap();
                    *guard += item.chunk.chunk_size;
                    updater(SophonProgress::Downloading {
                        downloaded_bytes: *guard,
                        total_bytes: total_compressed,
                    });
                }

                let ready_files: Vec<(SophonManifestAssetProperty, PathBuf)> = {
                    let mut map = chunk_to_files.lock().unwrap();
                    if let Some(entries) = map.remove(&item.chunk.chunk_name) {
                        entries
                            .into_iter()
                            .filter_map(|(file, tmp_dir, pending)| {
                                let mut count = pending.lock().unwrap();
                                *count -= 1;
                                if *count == 0 { Some((file, tmp_dir)) } else { None }
                            })
                            .collect()
                    } else {
                        vec![]
                    }
                };

                for entry in ready_files {
                    let _ = assemble_tx.send(entry);
                }

                Ok(())
            }
        })
        .buffer_unordered(DOWNLOAD_CONCURRENCY)
        .collect()
        .await;
    drop(assemble_tx);
    results.into_iter().find(|r| r.is_err()).transpose()?;
    assembly_task
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    Ok(())
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

    let bytes = resp.bytes().await?;

    if bytes.len() as u64 != chunk.chunk_size {
        return Err(format!(
            "Downloaded size mismatch for {}: expected {}, got {}",
            chunk.chunk_name,
            chunk.chunk_size,
            bytes.len()
        )
        .into());
    }

    if !chunk.chunk_compressed_hash_md5.is_empty() {
        let actual = md5_hex(&bytes);
        if actual != chunk.chunk_compressed_hash_md5 {
            return Err(format!(
                "Compressed MD5 mismatch for {}: expected {}, got {actual}",
                chunk.chunk_name, chunk.chunk_compressed_hash_md5
            )
            .into());
        }
    }

    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || fs::write(&dest, &bytes)).await??;

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

fn assemble_file(
    file: &SophonManifestAssetProperty,
    game_dir: &Path,
    chunks_dir: &Path,
    temp_dir: &Path,
    chunk_refcounts: &Mutex<HashMap<String, usize>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let target_path = game_dir.join(&file.asset_name);
    let tmp_path = temp_dir.join(format!("{}.tmp", md5_hex(file.asset_name.as_bytes())));

    if target_path.exists() && check_file_md5(&target_path, file.asset_size, &file.asset_hash_md5)
    {
        let mut map = chunk_refcounts.lock().unwrap();
        for chunk in &file.asset_chunks {
            if let Some(count) = map.get_mut(&chunk.chunk_name) {
                *count -= 1;
                if *count == 0 {
                    let _ = fs::remove_file(chunks_dir.join(chunk_filename(chunk)));
                }
            }
        }
        return Ok(());
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

    for chunk in &file.asset_chunks {
        let chunk_path = chunks_dir.join(chunk_filename(chunk));
        let decompressed = decompress_chunk(&chunk_path)?;

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

        let mut map = chunk_refcounts.lock().unwrap();
        if let Some(count) = map.get_mut(&chunk.chunk_name) {
            *count -= 1;
            if *count == 0 {
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
        let actual = file_md5_hex(&tmp_path)?;
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
    format!("{:x}", md5::compute(data))
}

fn file_md5_hex(path: &Path) -> std::io::Result<String> {
    let data = fs::read(path)?;
    Ok(format!("{:x}", md5::compute(data)))
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
