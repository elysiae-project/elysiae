use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use tauri_plugin_log::log;

use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetProperty;

const DATA_DIR: &str = "ZenlessZoneZero_Data";
const AUDIO_LANG_LAUNCHER_FILE: &str = "audio_lang_launcher";
const AUDIO_LANG_FILE: &str = "audio_lang";
const KDEL_RESOURCE_FILE: &str = "KDelResource";

pub fn filter_nap_asset_list(game_dir: &Path, assets: &mut Vec<SophonManifestAssetProperty>) {
    let kdel_path = game_dir.join(format!("{DATA_DIR}/Persistent/{KDEL_RESOURCE_FILE}"));
    if !kdel_path.exists() {
        return;
    }

    match fs::read_to_string(&kdel_path) {
        Ok(content) => {
            let first_line = content.lines().next().unwrap_or("");
            log::warn!(
                "nap KDelResource found but filtering is not applied (placeholder). Content: {:?}",
                first_line
            );
        }
        Err(e) => {
            log::warn!("Failed to read KDelResource: {}", e);
        }
    }

    let _ = assets;
}

pub fn write_nap_audio_lang_records(game_dir: &Path, vo_langs: &[String]) -> std::io::Result<()> {
    let persistent_dir = game_dir.join(format!("{DATA_DIR}/Persistent"));
    fs::create_dir_all(&persistent_dir)?;

    write_lang_file(
        &persistent_dir.join(AUDIO_LANG_LAUNCHER_FILE),
        vo_langs,
        locale_code_to_audio_lang_name,
    )?;

    write_lang_file(
        &persistent_dir.join(AUDIO_LANG_FILE),
        vo_langs,
        locale_code_to_abbrev_lang_name,
    )?;

    Ok(())
}

fn write_lang_file(
    path: &Path,
    vo_langs: &[String],
    mapper: fn(&str) -> Option<&'static str>,
) -> std::io::Result<()> {
    let mut existing: Vec<String> = Vec::new();
    if path.exists() {
        if let Ok(content) = fs::read_to_string(path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    existing.push(trimmed.to_string());
                }
            }
        }
    }

    for lang in vo_langs {
        if let Some(name) = mapper(lang) {
            if !existing.iter().any(|e| e == name) {
                existing.push(name.to_string());
            }
        }
    }

    let mut content = String::new();
    for name in &existing {
        content.push_str(name);
        content.push('\n');
    }

    let mut file = File::create(path)?;
    file.write_all(content.as_bytes())?;

    Ok(())
}

pub fn locale_code_to_audio_lang_name(locale: &str) -> Option<&'static str> {
    match locale {
        "zh-cn" | "cn" => Some("Chinese"),
        "en-us" | "en" => Some("English(US)"),
        "ja-jp" | "jp" => Some("Japanese"),
        "ko-kr" | "kr" => Some("Korean"),
        _ => None,
    }
}

pub fn locale_code_to_abbrev_lang_name(locale: &str) -> Option<&'static str> {
    match locale {
        "zh-cn" | "cn" => Some("Cn"),
        "en-us" | "en" => Some("En"),
        "ja-jp" | "jp" => Some("Jp"),
        "ko-kr" | "kr" => Some("Kr"),
        _ => None,
    }
}
