use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use tauri_plugin_log::log;

use super::write_lang_file;
use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetProperty;

const AUDIO_LANG_FILE: &str = "AudioLaucherRecord.txt";
const CONFIG_INI: &str = "config.ini";
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

pub fn write_config_ini(game_dir: &Path, game_version: &str) -> std::io::Result<()> {
    let config_path = game_dir.join(CONFIG_INI);
    let general_section = "[General]";
    let base_keys = &[
        ("channel", "1"),
        ("sub_channel", "6"),
        ("cps", "mihoyohkrpg_oversea"),
        ("game_version", game_version),
        ("sdk_version", ""),
    ];

    if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        let mut lines: Vec<String> = content.lines().map(String::from).collect();
        let mut in_general = false;
        let mut found_keys: std::collections::HashSet<&str> = std::collections::HashSet::new();

        for line in &mut lines {
            let trimmed = line.trim();
            if trimmed == general_section {
                in_general = true;
                continue;
            }
            if in_general && trimmed.starts_with('[') {
                in_general = false;
            }
            if in_general && let Some(eq_pos) = trimmed.find('=') {
                let key = trimmed[..eq_pos].trim().to_string();
                for &(k, v) in base_keys {
                    if key == k {
                        *line = format!("{k}={v}");
                        found_keys.insert(k);
                    }
                }
            }
        }

        let mut insert_idx = 0;
        let mut in_general_section = false;
        for (i, line) in lines.iter().enumerate() {
            if line.trim() == general_section {
                in_general_section = true;
                insert_idx = i + 1;
            } else if in_general_section && line.trim().starts_with('[') {
                insert_idx = i;
                break;
            } else if in_general_section {
                insert_idx = i + 1;
            }
        }

        if in_general_section {
            let mut to_insert: Vec<String> = Vec::new();
            for &(k, v) in base_keys {
                if !found_keys.contains(k) {
                    to_insert.push(format!("{k}={v}"));
                }
            }
            for (offset, line) in to_insert.into_iter().enumerate() {
                lines.insert(insert_idx + offset, line);
            }
        } else {
            lines.push(String::new());
            lines.push(general_section.to_string());
            for &(k, v) in base_keys {
                lines.push(format!("{k}={v}"));
            }
        }

        let mut out = lines.join("\n");
        if !out.ends_with('\n') {
            out.push('\n');
        }
        fs::write(&config_path, out)
    } else {
        let mut f = File::create(&config_path)?;
        writeln!(f, "{}", general_section)?;
        for &(k, v) in base_keys {
            writeln!(f, "{k}={v}")?;
        }
        Ok(())
    }
}

pub fn write_app_info(game_dir: &Path) -> std::io::Result<()> {
    let app_info_path = game_dir.join(APP_INFO_FILE);
    if let Some(parent) = app_info_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&app_info_path, "Cognosphere\nhkrpg_global\n")
}
