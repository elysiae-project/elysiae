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

/// ZZZ does not currently require asset-level filtering.
/// Audio language filtering is handled at the installer level
/// via `filter_nap_installers` and `write_nap_audio_lang_records`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // -----------------------------------------------------------------------
    // filter_nap_asset_list (no-op)
    // -----------------------------------------------------------------------
    #[test]
    fn test_filter_nap_asset_list_does_not_modify() {
        let dir = tempfile::tempdir().unwrap();

        let mut assets = vec![SophonManifestAssetProperty {
            asset_name: "ZenlessZoneZero_Data/audio/file.pck".into(),
            asset_chunks: vec![],
            asset_type: 0,
            asset_size: 100,
            asset_hash_md5: "abc".into(),
        }];

        filter_nap_asset_list(dir.path(), &mut assets);
        assert_eq!(assets.len(), 1);
    }

    // -----------------------------------------------------------------------
    // locale_code_to_audio_lang_name (tested via super::)
    // -----------------------------------------------------------------------
    #[test]
    fn test_nap_locale_code_to_audio_lang_name_zh_cn() {
        assert_eq!(locale_code_to_audio_lang_name("zh-cn"), Some("Chinese"));
    }

    #[test]
    fn test_nap_locale_code_to_audio_lang_name_cn() {
        assert_eq!(locale_code_to_audio_lang_name("cn"), Some("Chinese"));
    }

    #[test]
    fn test_nap_locale_code_to_audio_lang_name_en_us() {
        assert_eq!(locale_code_to_audio_lang_name("en-us"), Some("English(US)"));
    }

    #[test]
    fn test_nap_locale_code_to_audio_lang_name_en() {
        assert_eq!(locale_code_to_audio_lang_name("en"), Some("English(US)"));
    }

    #[test]
    fn test_nap_locale_code_to_audio_lang_name_ja_jp() {
        assert_eq!(locale_code_to_audio_lang_name("ja-jp"), Some("Japanese"));
    }

    #[test]
    fn test_nap_locale_code_to_audio_lang_name_jp() {
        assert_eq!(locale_code_to_audio_lang_name("jp"), Some("Japanese"));
    }

    #[test]
    fn test_nap_locale_code_to_audio_lang_name_ko_kr() {
        assert_eq!(locale_code_to_audio_lang_name("ko-kr"), Some("Korean"));
    }

    #[test]
    fn test_nap_locale_code_to_audio_lang_name_kr() {
        assert_eq!(locale_code_to_audio_lang_name("kr"), Some("Korean"));
    }

    #[test]
    fn test_nap_locale_code_to_audio_lang_name_unknown() {
        assert_eq!(locale_code_to_audio_lang_name("fr-fr"), None);
        assert_eq!(locale_code_to_audio_lang_name(""), None);
    }

    // -----------------------------------------------------------------------
    // locale_code_to_abbrev_lang_name (tested via super::)
    // -----------------------------------------------------------------------
    #[test]
    fn test_locale_code_to_abbrev_lang_name_zh_cn() {
        assert_eq!(locale_code_to_abbrev_lang_name("zh-cn"), Some("Cn"));
    }

    #[test]
    fn test_locale_code_to_abbrev_lang_name_cn() {
        assert_eq!(locale_code_to_abbrev_lang_name("cn"), Some("Cn"));
    }

    #[test]
    fn test_locale_code_to_abbrev_lang_name_en_us() {
        assert_eq!(locale_code_to_abbrev_lang_name("en-us"), Some("En"));
    }

    #[test]
    fn test_locale_code_to_abbrev_lang_name_en() {
        assert_eq!(locale_code_to_abbrev_lang_name("en"), Some("En"));
    }

    #[test]
    fn test_locale_code_to_abbrev_lang_name_ja_jp() {
        assert_eq!(locale_code_to_abbrev_lang_name("ja-jp"), Some("Jp"));
    }

    #[test]
    fn test_locale_code_to_abbrev_lang_name_jp() {
        assert_eq!(locale_code_to_abbrev_lang_name("jp"), Some("Jp"));
    }

    #[test]
    fn test_locale_code_to_abbrev_lang_name_ko_kr() {
        assert_eq!(locale_code_to_abbrev_lang_name("ko-kr"), Some("Kr"));
    }

    #[test]
    fn test_locale_code_to_abbrev_lang_name_kr() {
        assert_eq!(locale_code_to_abbrev_lang_name("kr"), Some("Kr"));
    }

    #[test]
    fn test_locale_code_to_abbrev_lang_name_unknown() {
        assert_eq!(locale_code_to_abbrev_lang_name("fr-fr"), None);
        assert_eq!(locale_code_to_abbrev_lang_name(""), None);
    }

    // -----------------------------------------------------------------------
    // read_kdel_resource_matching_fields
    // -----------------------------------------------------------------------
    #[test]
    fn test_read_kdel_resource_matching_fields_with_pipe_separator() {
        let dir = tempfile::tempdir().unwrap();
        let kdel_dir = dir.path().join("ZenlessZoneZero_Data/Persistent");
        fs::create_dir_all(&kdel_dir).unwrap();
        fs::write(kdel_dir.join("KDelResource"), "field_a|field_B|field_c").unwrap();

        let result = read_kdel_resource_matching_fields(dir.path());
        assert!(result.is_some());
        let fields = result.unwrap();
        // Case-insensitive dedup: field_B and field_b would be same if both present
        assert_eq!(fields.len(), 3);
        assert!(fields.iter().any(|f| f.eq_ignore_ascii_case("field_a")));
        assert!(fields.iter().any(|f| f.eq_ignore_ascii_case("field_B")));
        assert!(fields.iter().any(|f| f.eq_ignore_ascii_case("field_c")));
    }

    #[test]
    fn test_read_kdel_resource_matching_fields_with_semicolon() {
        let dir = tempfile::tempdir().unwrap();
        let kdel_dir = dir.path().join("ZenlessZoneZero_Data/Persistent");
        fs::create_dir_all(&kdel_dir).unwrap();
        fs::write(kdel_dir.join("KDelResource"), "game;test").unwrap();

        let result = read_kdel_resource_matching_fields(dir.path());
        assert!(result.is_some());
        let fields = result.unwrap();
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn test_read_kdel_resource_matching_fields_with_dollar() {
        let dir = tempfile::tempdir().unwrap();
        let kdel_dir = dir.path().join("ZenlessZoneZero_Data/Persistent");
        fs::create_dir_all(&kdel_dir).unwrap();
        fs::write(kdel_dir.join("KDelResource"), "alpha$beta$gamma").unwrap();

        let result = read_kdel_resource_matching_fields(dir.path());
        assert!(result.is_some());
        let fields = result.unwrap();
        assert_eq!(fields.len(), 3);
    }

    #[test]
    fn test_read_kdel_resource_matching_fields_with_hash() {
        let dir = tempfile::tempdir().unwrap();
        let kdel_dir = dir.path().join("ZenlessZoneZero_Data/Persistent");
        fs::create_dir_all(&kdel_dir).unwrap();
        fs::write(kdel_dir.join("KDelResource"), "x#y#z").unwrap();

        let result = read_kdel_resource_matching_fields(dir.path());
        assert!(result.is_some());
        let fields = result.unwrap();
        assert_eq!(fields.len(), 3);
    }

    #[test]
    fn test_read_kdel_resource_matching_fields_with_at_and_plus() {
        let dir = tempfile::tempdir().unwrap();
        let kdel_dir = dir.path().join("ZenlessZoneZero_Data/Persistent");
        fs::create_dir_all(&kdel_dir).unwrap();
        fs::write(kdel_dir.join("KDelResource"), "a@b+c").unwrap();

        let result = read_kdel_resource_matching_fields(dir.path());
        assert!(result.is_some());
        let fields = result.unwrap();
        assert_eq!(fields.len(), 3);
    }

    #[test]
    fn test_read_kdel_resource_matching_fields_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        // No KDelResource file created

        let result = read_kdel_resource_matching_fields(dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_read_kdel_resource_matching_fields_empty() {
        let dir = tempfile::tempdir().unwrap();
        let kdel_dir = dir.path().join("ZenlessZoneZero_Data/Persistent");
        fs::create_dir_all(&kdel_dir).unwrap();
        fs::write(kdel_dir.join("KDelResource"), "").unwrap();

        let result = read_kdel_resource_matching_fields(dir.path());
        // Empty file -> content.lines().next() will return None -> returns None
        assert!(result.is_none());
    }

    #[test]
    fn test_read_kdel_resource_matching_fields_dedup_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let kdel_dir = dir.path().join("ZenlessZoneZero_Data/Persistent");
        fs::create_dir_all(&kdel_dir).unwrap();
        fs::write(kdel_dir.join("KDelResource"), "Game|GAME|game").unwrap();

        let result = read_kdel_resource_matching_fields(dir.path());
        assert!(result.is_some());
        let fields = result.unwrap();
        // All three are case-insensitively equal, so only one should remain
        assert_eq!(fields.len(), 1);
    }

    // -----------------------------------------------------------------------
    // filter_nap_installers (tested with empty vec since InstallerData is in
    // another module)
    // -----------------------------------------------------------------------
    #[test]
    fn test_filter_nap_installers_empty_vec() {
        let dir = tempfile::tempdir().unwrap();

        let mut installers: Vec<InstallerData> = Vec::new();
        filter_nap_installers(dir.path(), &mut installers);
        assert!(installers.is_empty());
    }

    #[test]
    fn test_filter_nap_installers_no_kdel_file() {
        let dir = tempfile::tempdir().unwrap();
        // No KDelResource file

        let mut installers: Vec<InstallerData> = Vec::new();
        filter_nap_installers(dir.path(), &mut installers);
        // Should not crash, just return early
        assert!(installers.is_empty());
    }

    // -----------------------------------------------------------------------
    // write_nap_audio_lang_records
    // -----------------------------------------------------------------------
    #[test]
    fn test_write_nap_audio_lang_records_creates_both_files() {
        let dir = tempfile::tempdir().unwrap();

        let vo_langs = vec![
            "zh-cn".to_string(),
            "en-us".to_string(),
            "ja-jp".to_string(),
        ];
        write_nap_audio_lang_records(dir.path(), &vo_langs).unwrap();

        let persistent_dir = dir.path().join("ZenlessZoneZero_Data/Persistent");
        assert!(persistent_dir.exists());

        // audio_lang_launcher uses full names
        let launcher_content =
            fs::read_to_string(persistent_dir.join("audio_lang_launcher")).unwrap();
        assert_eq!(launcher_content, "Chinese\nEnglish(US)\nJapanese\n");

        // audio_lang uses abbreviations
        let lang_content = fs::read_to_string(persistent_dir.join("audio_lang")).unwrap();
        assert_eq!(lang_content, "Cn\nEn\nJp\n");
    }

    #[test]
    fn test_write_nap_audio_lang_records_empty_vo_langs() {
        let dir = tempfile::tempdir().unwrap();

        write_nap_audio_lang_records(dir.path(), &[]).unwrap();

        let persistent_dir = dir.path().join("ZenlessZoneZero_Data/Persistent");
        let launcher_content =
            fs::read_to_string(persistent_dir.join("audio_lang_launcher")).unwrap();
        assert!(launcher_content.is_empty());

        let lang_content = fs::read_to_string(persistent_dir.join("audio_lang")).unwrap();
        assert!(lang_content.is_empty());
    }
}
