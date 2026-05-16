use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use serde::Serialize;
use tauri_plugin_log::log;

use super::write_lang_file;
use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetProperty;

const ALL_AUDIO_LANGUAGES: &[&str] = &["Chinese", "English(US)", "Japanese", "Korean"];

const AUDIO_LANG_FILE: &str = "audio_lang_14";

pub fn filter_hk4e_asset_list(
    game_dir: &Path,
    assets: &mut Vec<SophonManifestAssetProperty>,
    vo_langs: &[String],
) {
    let persistent_dir = find_hk4e_persistent_dir(game_dir);

    let installed_langs = read_installed_audio_langs(&persistent_dir, vo_langs);

    let ignored_langs: Vec<&str> = ALL_AUDIO_LANGUAGES
        .iter()
        .filter(|lang| !installed_langs.iter().any(|installed| installed == **lang))
        .copied()
        .collect();

    if ignored_langs.is_empty() {
        return;
    }

    let patterns: Vec<String> = ignored_langs
        .iter()
        .map(|lang| format!("/{lang}/"))
        .collect();

    let original_len = assets.len();
    assets.retain(|asset| {
        let asset_lower = asset.asset_name.to_lowercase();

        for pattern in &patterns {
            if asset_lower.contains(&pattern.to_lowercase()) {
                log::warn!("Filtered unneeded audio asset: {}", asset.asset_name);
                return false;
            }
        }

        if asset_lower.ends_with("ctable_streaming.dat") {
            log::warn!("Filtered ctable asset: {}", asset.asset_name);
            return false;
        }

        true
    });

    let filtered = original_len - assets.len();
    if filtered > 0 {
        log::warn!("hk4e filter removed {} assets", filtered);
    }
}

pub fn write_audio_lang_record(game_dir: &Path, vo_langs: &[String]) -> std::io::Result<()> {
    let persistent_dir = find_hk4e_persistent_dir(game_dir);
    fs::create_dir_all(&persistent_dir)?;

    write_lang_file(
        &persistent_dir.join(AUDIO_LANG_FILE),
        vo_langs,
        locale_code_to_audio_lang_name,
    )
}

fn locale_code_to_audio_lang_name(locale: &str) -> Option<&'static str> {
    match locale {
        "zh-cn" | "cn" => Some("Chinese"),
        "en-us" | "en" => Some("English(US)"),
        "ja-jp" | "jp" => Some("Japanese"),
        "ko-kr" | "kr" => Some("Korean"),
        _ => None,
    }
}

#[derive(Serialize)]
#[allow(non_snake_case)]
struct PkgVersionEntry {
    remoteName: String,
    md5: String,
    #[serde(rename = "fileSize")]
    file_size: u64,
}

pub fn write_pkg_version_from_manifest(
    game_dir: &Path,
    assets: &[SophonManifestAssetProperty],
    vo_langs: &[String],
) -> std::io::Result<()> {
    write_single_pkg_version(game_dir, "pkg_version", assets)?;

    for lang in vo_langs {
        if let Some(lang_name) = locale_code_to_audio_lang_name(lang) {
            let filename = format!("Audio_{lang_name}_pkg_version");
            let filtered: Vec<SophonManifestAssetProperty> = assets
                .iter()
                .filter(|a| {
                    let lower = a.asset_name.to_lowercase();
                    lower.contains(&format!("/{lang_name}/").to_lowercase())
                })
                .cloned()
                .collect();
            write_single_pkg_version(game_dir, &filename, &filtered)?;
        }
    }

    Ok(())
}

fn find_hk4e_persistent_dir(game_dir: &Path) -> std::path::PathBuf {
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

fn read_installed_audio_langs(persistent_dir: &Path, vo_langs: &[String]) -> Vec<String> {
    if let Ok(entries) = fs::read_dir(persistent_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("audio_lang_") {
                let path = entry.path();
                if let Ok(content) = fs::read_to_string(&path) {
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

    vo_langs
        .iter()
        .filter_map(|lang| locale_code_to_audio_lang_name(lang).map(|s| s.to_string()))
        .collect()
}

fn write_single_pkg_version(
    game_dir: &Path,
    filename: &str,
    assets: &[SophonManifestAssetProperty],
) -> std::io::Result<()> {
    let path = game_dir.join(filename);
    let mut file = File::create(&path)?;

    for asset in assets {
        if asset.is_directory() {
            continue;
        }
        let entry = PkgVersionEntry {
            remoteName: asset.asset_name.clone(),
            md5: asset.asset_hash_md5.clone(),
            file_size: asset.asset_size,
        };
        let mut line = serde_json::to_string(&entry)?;
        line.push('\n');
        file.write_all(line.as_bytes())?;
    }

    Ok(())
}
