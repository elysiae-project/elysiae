use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;

use tauri_plugin_log::log;

use super::write_lang_file;
use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetProperty;

const AUDIO_LANG_FILE: &str = "AudioLaucherRecord.txt";

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
