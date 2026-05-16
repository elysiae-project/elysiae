use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use tauri_plugin_log::log;

use super::write_lang_file;
use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetProperty;

const AUDIO_LANG_FILE: &str = "AudioLaucherRecord.txt";
const APP_INFO_FILE: &str = "StarRail_Data/app.info";

pub fn filter_hkrpg_asset_list(game_dir: &Path, assets: &mut Vec<SophonManifestAssetProperty>) {
    let blacklist_path = game_dir.join("StarRail_Data/Persistent/DownloadBlacklist.json");
    if !blacklist_path.exists() {
        return;
    }

    let file = match File::open(&blacklist_path) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("Failed to open DownloadBlacklist.json: {}", e);
            return;
        }
    };

    let mut blacklist: Vec<String> = Vec::new();

    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                log::warn!("Failed to read line from DownloadBlacklist.json: {}", e);
                continue;
            }
        };

        let file_name = match extract_filename(&line) {
            Some(name) => name,
            None => continue,
        };

        let normalized = file_name.replace('\\', "/");
        blacklist.push(normalized.clone());

        add_both_persistent_or_streaming_assets(&normalized, &mut blacklist);
    }

    if blacklist.is_empty() {
        return;
    }

    let original_len = assets.len();
    assets.retain(|asset| {
        let asset_lower = asset.asset_name.to_lowercase();
        for entry in &blacklist {
            if asset_lower.contains(&entry.to_lowercase()) {
                log::warn!("Filtered blacklisted asset: {}", asset.asset_name);
                return false;
            }
        }
        true
    });

    let filtered = original_len - assets.len();
    if filtered > 0 {
        log::warn!("hkrpg blacklist filter removed {} assets", filtered);
    }
}

pub fn write_audio_lang_record(game_dir: &Path, vo_langs: &[String]) -> std::io::Result<()> {
    let persistent_dir = game_dir.join("StarRail_Data/Persistent");
    fs::create_dir_all(&persistent_dir)?;

    write_lang_file(
        &persistent_dir.join(AUDIO_LANG_FILE),
        vo_langs,
        locale_code_to_audio_lang_name,
    )
}

fn locale_code_to_audio_lang_name(locale: &str) -> Option<&'static str> {
    match locale {
        "zh-cn" | "zh-tw" | "cn" => Some("Chinese"),
        "en-us" | "en" => Some("English(US)"),
        "ja-jp" | "jp" => Some("Japanese"),
        "ko-kr" | "kr" => Some("Korean"),
        _ => None,
    }
}

fn extract_filename(line: &str) -> Option<String> {
    let marker = "\"fileName\":\"";
    let start = line.find(marker)? + marker.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn add_both_persistent_or_streaming_assets(path: &str, blacklist: &mut Vec<String>) {
    let streaming_prefix = "StarRail_Data/StreamingAssets/";
    let persistent_prefix = "StarRail_Data/Persistent/";

    if let Some(rest) = path.strip_prefix(streaming_prefix) {
        blacklist.push(format!("{}{}", persistent_prefix, rest));
    } else if let Some(rest) = path.strip_prefix(persistent_prefix) {
        blacklist.push(format!("{}{}", streaming_prefix, rest));
    }
}

pub fn write_app_info(game_dir: &Path) -> std::io::Result<()> {
    let app_info_path = game_dir.join(APP_INFO_FILE);
    if let Some(parent) = app_info_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&app_info_path, "Cognosphere\nhkrpg_global\n")
}

pub fn write_binary_version_files(game_dir: &Path) -> std::io::Result<()> {
    let bv_path = game_dir.join("StarRail_Data/StreamingAssets/BinaryVersion.bytes");
    if !bv_path.exists() {
        log::warn!("write_binary_version_files: BinaryVersion.bytes not found, skipping");
        return Ok(());
    }

    let mut buf = Vec::new();
    File::open(&bv_path)?.read_to_end(&mut buf)?;
    if buf.len() < 16 {
        log::warn!(
            "write_binary_version_files: BinaryVersion.bytes too short ({} bytes), skipping",
            buf.len()
        );
        return Ok(());
    }

    // Collapse: Span<byte> bufferSpan = buffer.AsSpan()[..^3];
    let data = &buf[..buf.len().saturating_sub(3)];
    if data.len() < 40 {
        log::warn!(
            "write_binary_version_files: BinaryVersion.bytes data too short after trim ({} bytes), skipping",
            data.len()
        );
        return Ok(());
    }

    // Collapse: Span<byte> hashSpan = bufferSpan[^36..^4];
    let hash_end = data.len().saturating_sub(4);
    let hash_start = hash_end.saturating_sub(36);
    let hash_str = std::str::from_utf8(&data[hash_start..hash_end])
        .unwrap_or("")
        .to_string();

    // Collapse: GetVersionNumber reads BigEndian uint16 str_len, skips (2+str_len),
    // then patch(u32), major(u32), minor(u32)
    let (major, minor, patch) = if data.len() > 2 {
        let str_len = u16::from_be_bytes([data[0], data[1]]) as usize;
        let version_start = 2 + str_len;
        if data.len() >= version_start + 12 {
            let patch = u32::from_be_bytes([
                data[version_start],
                data[version_start + 1],
                data[version_start + 2],
                data[version_start + 3],
            ]);
            let major = u32::from_be_bytes([
                data[version_start + 4],
                data[version_start + 5],
                data[version_start + 6],
                data[version_start + 7],
            ]);
            let minor = u32::from_be_bytes([
                data[version_start + 8],
                data[version_start + 9],
                data[version_start + 10],
                data[version_start + 11],
            ]);
            (major, minor, patch)
        } else {
            log::warn!(
                "write_binary_version_files: could not parse version numbers, writing hash-only files"
            );
            (0, 0, 0)
        }
    } else {
        (0, 0, 0)
    };

    let persistent_dir = game_dir.join("StarRail_Data/Persistent");
    fs::create_dir_all(&persistent_dir)?;

    fs::write(persistent_dir.join("AppIdentity.txt"), &hash_str)?;
    fs::write(persistent_dir.join("DownloadedFullAssets.txt"), &hash_str)?;
    fs::write(
        persistent_dir.join("InstallVersion.bin"),
        format!("{},{major}.{minor}.{patch}", hash_str),
    )?;

    log::info!(
        "write_binary_version_files: wrote AppIdentity.txt, DownloadedFullAssets.txt, InstallVersion.bin (hash={}, version={major}.{minor}.{patch})",
        hash_str
    );
    Ok(())
}
