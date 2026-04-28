use std::fs;
use std::path::Path;

use tauri_plugin_log::log;

use super::write_lang_file;
use crate::commands::sophon_downloader::game_installer::installer::InstallerData;
use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetProperty;

const DATA_DIR: &str = "ZenlessZoneZero_Data";
const AUDIO_LANG_LAUNCHER_FILE: &str = "audio_lang_launcher";
const AUDIO_LANG_FILE: &str = "audio_lang";
const KDEL_RESOURCE_FILE: &str = "KDelResource";
const KDEL_SEPARATORS: &[char] = &['|', ';', ',', '$', '#', '@', '+', ' '];

pub fn filter_nap_asset_list(game_dir: &Path, assets: &mut Vec<SophonManifestAssetProperty>) {
    let _ = (game_dir, assets);
}

pub fn filter_nap_installers(game_dir: &Path, installer_data: &mut Vec<InstallerData>) {
    let kdel_fields = match read_kdel_resource_matching_fields(game_dir) {
        Some(fields) if !fields.is_empty() => fields,
        _ => return,
    };

    let original_len = installer_data.len();
    installer_data.retain(|data| {
        if kdel_fields
            .iter()
            .any(|f| f.eq_ignore_ascii_case(&data.matching_field))
        {
            log::warn!(
                "nap filter removing installer with matching_field: {}",
                data.matching_field
            );
            return false;
        }
        true
    });

    let filtered = original_len - installer_data.len();
    if filtered > 0 {
        log::warn!("nap KDelResource filter removed {} installers", filtered);
    }
}

fn read_kdel_resource_matching_fields(game_dir: &Path) -> Option<Vec<String>> {
    let kdel_path = game_dir.join(format!("{DATA_DIR}/Persistent/{KDEL_RESOURCE_FILE}"));

    if !kdel_path.exists() {
        return None;
    }

    let content = fs::read_to_string(&kdel_path).ok()?;
    let first_line = content.lines().next()?;

    let mut fields: Vec<String> = Vec::new();
    for token in first_line.split(KDEL_SEPARATORS) {
        let trimmed = token.trim_matches(KDEL_SEPARATORS);
        if !trimmed.is_empty() && !fields.iter().any(|f| f.eq_ignore_ascii_case(trimmed)) {
            fields.push(trimmed.to_string());
        }
    }

    Some(fields)
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

fn locale_code_to_audio_lang_name(locale: &str) -> Option<&'static str> {
    match locale {
        "zh-cn" | "cn" => Some("Chinese"),
        "en-us" | "en" => Some("English(US)"),
        "ja-jp" | "jp" => Some("Japanese"),
        "ko-kr" | "kr" => Some("Korean"),
        _ => None,
    }
}

fn locale_code_to_abbrev_lang_name(locale: &str) -> Option<&'static str> {
    match locale {
        "zh-cn" | "cn" => Some("Cn"),
        "en-us" | "en" => Some("En"),
        "ja-jp" | "jp" => Some("Jp"),
        "ko-kr" | "kr" => Some("Kr"),
        _ => None,
    }
}
