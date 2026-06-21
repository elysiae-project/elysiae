use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use tauri_plugin_log::log;

use super::write_lang_file;
use crate::commands::sophon_downloader::proto_parse::SophonManifestAssetProperty;

const AUDIO_LANG_FILE: &str = "AudioLaucherRecord.txt";
const GAME_DATA_DIR: &str = "\x53\x74\x61\x72\x52\x61\x69\x6c\x5f\x44\x61\x74\x61";
const APP_VENDOR: &str = "\x43\x6f\x67\x6e\x6f\x73\x70\x68\x65\x72\x65";

pub fn filter_hkrpg_asset_list(game_dir: &Path, assets: &mut Vec<SophonManifestAssetProperty>) {
    let blacklist_path =
        game_dir.join(format!("{GAME_DATA_DIR}/Persistent/DownloadBlacklist.json"));
    if !blacklist_path.exists() {
        return;
    }

    let file = match File::open(&blacklist_path) {
        Ok(f) => f,
        Err(err) => {
            log::warn!("Failed to open DownloadBlacklist.json: {err}");
            return;
        }
    };

    let mut blacklist: Vec<String> = Vec::new();

    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(err) => {
                log::warn!("Failed to read line from DownloadBlacklist.json: {err}");
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

    let blacklist_lower: Vec<String> = blacklist.iter().map(|entry| entry.to_lowercase()).collect();

    let original_len = assets.len();
    assets.retain(|asset| {
        let asset_lower = asset.asset_name.to_lowercase();
        for entry in &blacklist_lower {
            if asset_lower.contains(entry) {
                let name = &asset.asset_name;
                log::warn!("Filtered blacklisted asset: {name}");
                return false;
            }
        }
        true
    });

    let filtered = original_len - assets.len();
    if filtered > 0 {
        log::warn!("hkrpg blacklist filter removed {filtered} assets");
    }
}

pub fn write_audio_lang_record(game_dir: &Path, vo_langs: &[String]) -> std::io::Result<()> {
    let persistent_dir = game_dir.join(format!("{GAME_DATA_DIR}/Persistent"));
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

/// Case-insensitive version of `str::strip_prefix`.
/// Converts both the string and prefix to lowercase for comparison, but
/// returns the original string slice (preserving case) on match.
fn strip_prefix_case_insensitive<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let prefix = prefix.to_lowercase();
    let lower_s = s.to_lowercase();
    if lower_s.starts_with(&prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn add_both_persistent_or_streaming_assets(path: &str, blacklist: &mut Vec<String>) {
    let streaming_prefix = format!("{GAME_DATA_DIR}/StreamingAssets/");
    let persistent_prefix = format!("{GAME_DATA_DIR}/Persistent/");

    if let Some(rest) = strip_prefix_case_insensitive(path, &streaming_prefix) {
        blacklist.push(format!("{persistent_prefix}{rest}"));
    } else if let Some(rest) = strip_prefix_case_insensitive(path, &persistent_prefix) {
        blacklist.push(format!("{streaming_prefix}{rest}"));
    }
}

pub fn write_app_info(game_dir: &Path) -> std::io::Result<()> {
    let app_info_path = game_dir.join(format!("{GAME_DATA_DIR}/app.info"));
    if let Some(parent) = app_info_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &app_info_path,
        format!("{APP_VENDOR}\nhkrpg_global\n").as_bytes(),
    )
}

pub fn write_binary_version_files(game_dir: &Path) -> std::io::Result<()> {
    let bv_path = game_dir.join(format!(
        "{GAME_DATA_DIR}/StreamingAssets/BinaryVersion.bytes"
    ));
    if !bv_path.exists() {
        log::error!("write_binary_version_files: BinaryVersion.bytes not found");
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "BinaryVersion.bytes not found",
        ));
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

    let data = &buf[..buf.len().saturating_sub(3)];
    if data.len() < 40 {
        log::warn!(
            "write_binary_version_files: BinaryVersion.bytes data too short after trim ({} bytes), skipping",
            data.len()
        );
        return Ok(());
    }

    let hash_end = data.len().saturating_sub(4);
    let hash_start = hash_end.saturating_sub(36);
    let hash_str = std::str::from_utf8(&data[hash_start..hash_end])
        .unwrap_or("")
        .to_string();

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

    let persistent_dir = game_dir.join(format!("{GAME_DATA_DIR}/Persistent"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn persistent_dir(base: &std::path::Path) -> std::path::PathBuf {
        base.join(format!("{GAME_DATA_DIR}/Persistent"))
    }

    fn streaming_dir(base: &std::path::Path) -> std::path::PathBuf {
        base.join(format!("{GAME_DATA_DIR}/StreamingAssets"))
    }

    // -----------------------------------------------------------------------
    // strip_prefix_case_insensitive
    // -----------------------------------------------------------------------
    #[test]
    fn test_strip_prefix_case_insensitive_matching() {
        let path = format!("{GAME_DATA_DIR}/StreamingAssets/foo/bar");
        let prefix = format!("{GAME_DATA_DIR}/StreamingAssets/");
        let result = strip_prefix_case_insensitive(&path, &prefix);
        assert_eq!(result, Some("foo/bar"));
    }

    #[test]
    fn test_strip_prefix_case_insensitive_case_insensitive() {
        let prefix = format!("{GAME_DATA_DIR}/StreamingAssets/");
        let result = strip_prefix_case_insensitive("STARRAIL_DATA/STREAMINGASSETS/foo", &prefix);
        assert_eq!(result, Some("foo"));
    }

    #[test]
    fn test_strip_prefix_case_insensitive_no_match() {
        let prefix = format!("{GAME_DATA_DIR}/");
        let result = strip_prefix_case_insensitive("Other_Data/foo", &prefix);
        assert_eq!(result, None);
    }

    #[test]
    fn test_strip_prefix_case_insensitive_empty_prefix() {
        let result = strip_prefix_case_insensitive("hello/world", "");
        assert_eq!(result, Some("hello/world"));
    }

    #[test]
    fn test_strip_prefix_case_insensitive_empty_string() {
        let prefix = format!("{GAME_DATA_DIR}/");
        let result = strip_prefix_case_insensitive("", &prefix);
        assert_eq!(result, None);
    }

    // -----------------------------------------------------------------------
    // extract_filename
    // -----------------------------------------------------------------------
    #[test]
    fn test_extract_filename_backslash() {
        let line = r#"{"fileName":"audio\voice\file.pck"}"#;
        assert_eq!(
            extract_filename(line),
            Some(r#"audio\voice\file.pck"#.to_string())
        );
    }

    #[test]
    fn test_extract_filename_no_marker() {
        let line = r#"{"otherField":"value"}"#;
        assert_eq!(extract_filename(line), None);
    }

    #[test]
    fn test_extract_filename_empty() {
        assert_eq!(extract_filename(""), None);
    }

    // -----------------------------------------------------------------------
    // add_both_persistent_or_streaming_assets
    // -----------------------------------------------------------------------
    #[test]
    fn test_add_both_persistent_or_streaming_streaming_to_persistent() {
        let mut blacklist = vec![];
        let path = format!("{GAME_DATA_DIR}/StreamingAssets/audio/voice.pck");
        add_both_persistent_or_streaming_assets(&path, &mut blacklist);
        assert_eq!(
            blacklist,
            vec![format!("{GAME_DATA_DIR}/Persistent/audio/voice.pck")]
        );
    }

    #[test]
    fn test_add_both_persistent_or_streaming_persistent_to_streaming() {
        let mut blacklist = vec![];
        let path = format!("{GAME_DATA_DIR}/Persistent/audio/voice.pck");
        add_both_persistent_or_streaming_assets(&path, &mut blacklist);
        assert_eq!(
            blacklist,
            vec![format!("{GAME_DATA_DIR}/StreamingAssets/audio/voice.pck")]
        );
    }

    #[test]
    fn test_add_both_persistent_or_streaming_unrelated() {
        let mut blacklist = vec![];
        add_both_persistent_or_streaming_assets("Other_Data/file.bin", &mut blacklist);
        assert!(blacklist.is_empty());
    }

    #[test]
    fn test_add_both_persistent_or_streaming_case_insensitive() {
        let mut blacklist = vec![];
        add_both_persistent_or_streaming_assets(
            "STARRAIL_DATA/STREAMINGASSETS/audio/voice.pck",
            &mut blacklist,
        );
        assert_eq!(
            blacklist,
            vec![format!("{GAME_DATA_DIR}/Persistent/audio/voice.pck")]
        );
    }

    // -----------------------------------------------------------------------
    // locale_code_to_audio_lang_name (tested via super::)
    // -----------------------------------------------------------------------
    #[test]
    fn test_hkrpg_locale_code_to_audio_lang_name_zh_cn() {
        assert_eq!(locale_code_to_audio_lang_name("zh-cn"), Some("Chinese"));
    }

    #[test]
    fn test_hkrpg_locale_code_to_audio_lang_name_zh_tw() {
        assert_eq!(locale_code_to_audio_lang_name("zh-tw"), Some("Chinese"));
    }

    #[test]
    fn test_hkrpg_locale_code_to_audio_lang_name_cn() {
        assert_eq!(locale_code_to_audio_lang_name("cn"), Some("Chinese"));
    }

    #[test]
    fn test_hkrpg_locale_code_to_audio_lang_name_en_us() {
        assert_eq!(locale_code_to_audio_lang_name("en-us"), Some("English(US)"));
    }

    #[test]
    fn test_hkrpg_locale_code_to_audio_lang_name_en() {
        assert_eq!(locale_code_to_audio_lang_name("en"), Some("English(US)"));
    }

    #[test]
    fn test_hkrpg_locale_code_to_audio_lang_name_ja_jp() {
        assert_eq!(locale_code_to_audio_lang_name("ja-jp"), Some("Japanese"));
    }

    #[test]
    fn test_hkrpg_locale_code_to_audio_lang_name_jp() {
        assert_eq!(locale_code_to_audio_lang_name("jp"), Some("Japanese"));
    }

    #[test]
    fn test_hkrpg_locale_code_to_audio_lang_name_ko_kr() {
        assert_eq!(locale_code_to_audio_lang_name("ko-kr"), Some("Korean"));
    }

    #[test]
    fn test_hkrpg_locale_code_to_audio_lang_name_kr() {
        assert_eq!(locale_code_to_audio_lang_name("kr"), Some("Korean"));
    }

    #[test]
    fn test_hkrpg_locale_code_to_audio_lang_name_unknown() {
        assert_eq!(locale_code_to_audio_lang_name("fr-fr"), None);
    }

    // -----------------------------------------------------------------------
    // filter_hkrpg_asset_list
    // -----------------------------------------------------------------------
    #[test]
    fn test_filter_hkrpg_asset_list_missing_blacklist_no_filtering() {
        let dir = tempfile::tempdir().unwrap();
        // No DownloadBlacklist.json created

        let mut assets = vec![SophonManifestAssetProperty {
            asset_name: format!("{GAME_DATA_DIR}/StreamingAssets/audio/voice.pck"),
            asset_chunks: vec![],
            asset_type: 0,
            asset_size: 100,
            asset_hash_md5: "abc".into(),
        }];

        filter_hkrpg_asset_list(dir.path(), &mut assets);
        assert_eq!(assets.len(), 1);
    }

    #[test]
    fn test_filter_hkrpg_asset_list_blacklist_filters_asset() {
        let dir = tempfile::tempdir().unwrap();
        let blacklist_dir = persistent_dir(dir.path());
        fs::create_dir_all(&blacklist_dir).unwrap();
        let blacklist_json =
            format!(r#"{{"fileName":"{GAME_DATA_DIR}/StreamingAssets/audio/voice_bad.pck"}}"#);
        fs::write(
            blacklist_dir.join("DownloadBlacklist.json"),
            &blacklist_json,
        )
        .unwrap();

        let mut assets = vec![
            SophonManifestAssetProperty {
                asset_name: format!("{GAME_DATA_DIR}/StreamingAssets/audio/voice_good.pck"),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
            SophonManifestAssetProperty {
                asset_name: format!("{GAME_DATA_DIR}/StreamingAssets/audio/voice_bad.pck"),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 200,
                asset_hash_md5: "def".into(),
            },
        ];

        filter_hkrpg_asset_list(dir.path(), &mut assets);
        assert_eq!(assets.len(), 1);
        assert_eq!(
            assets[0].asset_name,
            format!("{GAME_DATA_DIR}/StreamingAssets/audio/voice_good.pck")
        );
    }

    #[test]
    fn test_filter_hkrpg_asset_list_blacklist_backslash_normalized() {
        let dir = tempfile::tempdir().unwrap();
        let blacklist_dir = persistent_dir(dir.path());
        fs::create_dir_all(&blacklist_dir).unwrap();
        let blacklist_json = format!(
            r#"{{"fileName":"{dir}\StreamingAssets\audio\voice.pck"}}"#,
            dir = GAME_DATA_DIR
        );
        fs::write(
            blacklist_dir.join("DownloadBlacklist.json"),
            &blacklist_json,
        )
        .unwrap();

        let mut assets = vec![SophonManifestAssetProperty {
            asset_name: format!("{GAME_DATA_DIR}/StreamingAssets/audio/voice.pck"),
            asset_chunks: vec![],
            asset_type: 0,
            asset_size: 100,
            asset_hash_md5: "abc".into(),
        }];

        filter_hkrpg_asset_list(dir.path(), &mut assets);
        assert!(assets.is_empty());
    }

    #[test]
    fn test_filter_hkrpg_asset_list_empty_blacklist_no_filtering() {
        let dir = tempfile::tempdir().unwrap();
        let blacklist_dir = persistent_dir(dir.path());
        fs::create_dir_all(&blacklist_dir).unwrap();
        fs::write(blacklist_dir.join("DownloadBlacklist.json"), "").unwrap();

        let mut assets = vec![SophonManifestAssetProperty {
            asset_name: format!("{GAME_DATA_DIR}/StreamingAssets/audio/voice.pck"),
            asset_chunks: vec![],
            asset_type: 0,
            asset_size: 100,
            asset_hash_md5: "abc".into(),
        }];

        filter_hkrpg_asset_list(dir.path(), &mut assets);
        assert_eq!(assets.len(), 1);
    }

    #[test]
    fn test_filter_hkrpg_asset_list_malformed_line_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let blacklist_dir = persistent_dir(dir.path());
        fs::create_dir_all(&blacklist_dir).unwrap();
        fs::write(
            blacklist_dir.join("DownloadBlacklist.json"),
            "not a json line\n{\"fileName\":\"audio/bad.pck\"}",
        )
        .unwrap();

        let mut assets = vec![
            SophonManifestAssetProperty {
                asset_name: "audio/bad.pck".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
            SophonManifestAssetProperty {
                asset_name: "audio/good.pck".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 200,
                asset_hash_md5: "def".into(),
            },
        ];

        filter_hkrpg_asset_list(dir.path(), &mut assets);
        // The malformed line is skipped, but the valid one filters "audio/bad.pck"
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_name, "audio/good.pck");
    }

    // -----------------------------------------------------------------------
    // write_audio_lang_record
    // -----------------------------------------------------------------------
    #[test]
    fn test_hkrpg_write_audio_lang_record_creates_file() {
        let dir = tempfile::tempdir().unwrap();

        let vo_langs = vec!["zh-cn".to_string(), "en-us".to_string()];
        write_audio_lang_record(dir.path(), &vo_langs).unwrap();

        let record_path = dir
            .path()
            .join(format!("{GAME_DATA_DIR}/Persistent/AudioLaucherRecord.txt"));
        assert!(record_path.exists());

        let content = fs::read_to_string(&record_path).unwrap();
        assert_eq!(content, "Chinese\nEnglish(US)\n");
    }

    // -----------------------------------------------------------------------
    // write_app_info
    // -----------------------------------------------------------------------
    #[test]
    fn test_write_app_info_creates_file() {
        let dir = tempfile::tempdir().unwrap();

        write_app_info(dir.path()).unwrap();

        let app_info_path = dir.path().join(format!("{GAME_DATA_DIR}/app.info"));
        assert!(app_info_path.exists());

        let content = fs::read_to_string(&app_info_path).unwrap();
        assert_eq!(content, format!("{APP_VENDOR}\nhkrpg_global\n"));
    }

    // -----------------------------------------------------------------------
    // write_binary_version_files
    // -----------------------------------------------------------------------
    fn make_valid_binary_version_bytes() -> Vec<u8> {
        let hash = "abcdef0123456789abcdef0123456789abcd";
        let version_str = "OSRelWin64";
        let mut buf: Vec<u8> = Vec::new();

        // String length as u16 BE
        buf.extend_from_slice(&(version_str.len() as u16).to_be_bytes());
        // Version string bytes
        buf.extend_from_slice(version_str.as_bytes());
        // Patch, Major, Minor as u32 BE
        buf.extend_from_slice(&5u32.to_be_bytes());
        buf.extend_from_slice(&3u32.to_be_bytes());
        buf.extend_from_slice(&2u32.to_be_bytes());

        // Padding to push hash to correct position
        // data = buf[..buf.len()-3]; data.len() needs to have hash at data.len()-40
        // Current buf = 2 + 11 + 12 = 25 bytes
        // We want data.len() - 40 = 25, so data.len() = 65, buf.len() = 68
        let padding = 65 - buf.len();
        buf.resize(buf.len() + padding, 0xFF);

        // Hash at position 65-40=25 in data = position 25 in buf before trailing trim
        buf.extend_from_slice(hash.as_bytes());

        // 4 bytes after hash (before trailing trim)
        buf.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]);

        // 3 trailing bytes that will be trimmed
        buf.extend_from_slice(&[0xAA, 0xBB, 0xCC]);

        buf
    }

    #[test]
    fn test_write_binary_version_files_success() {
        let dir = tempfile::tempdir().unwrap();
        let bv_dir = streaming_dir(dir.path());
        fs::create_dir_all(&bv_dir).unwrap();

        let bytes = make_valid_binary_version_bytes();
        fs::write(bv_dir.join("BinaryVersion.bytes"), &bytes).unwrap();

        write_binary_version_files(dir.path()).unwrap();

        let persistent_dir = persistent_dir(dir.path());

        let app_id = fs::read_to_string(persistent_dir.join("AppIdentity.txt")).unwrap();
        assert_eq!(app_id, "abcdef0123456789abcdef0123456789abcd");

        let full_assets =
            fs::read_to_string(persistent_dir.join("DownloadedFullAssets.txt")).unwrap();
        assert_eq!(full_assets, "abcdef0123456789abcdef0123456789abcd");

        let install_ver = fs::read_to_string(persistent_dir.join("InstallVersion.bin")).unwrap();
        assert_eq!(install_ver, "abcdef0123456789abcdef0123456789abcd,3.2.5");
    }

    #[test]
    fn test_write_binary_version_files_missing() {
        let dir = tempfile::tempdir().unwrap();
        // No BinaryVersion.bytes created

        let result = write_binary_version_files(dir.path());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn test_write_binary_version_files_too_short() {
        let dir = tempfile::tempdir().unwrap();
        let bv_dir = streaming_dir(dir.path());
        fs::create_dir_all(&bv_dir).unwrap();
        // Write fewer than 16 bytes
        fs::write(bv_dir.join("BinaryVersion.bytes"), b"too short").unwrap();

        let result = write_binary_version_files(dir.path());
        assert!(result.is_ok());
    }
}
