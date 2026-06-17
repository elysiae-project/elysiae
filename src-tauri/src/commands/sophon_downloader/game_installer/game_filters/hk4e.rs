use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

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
    let persistent_dir = super::find_genshin_persistent_dir(game_dir);

    let installed_langs = read_installed_audio_langs(&persistent_dir, vo_langs);

    let ignored_langs: Vec<&str> = ALL_AUDIO_LANGUAGES
        .iter()
        .filter(|lang| !installed_langs.iter().any(|installed| installed == **lang))
        .copied()
        .collect();

    if ignored_langs.is_empty() {
        return;
    }

    let patterns_lower: Vec<String> = ignored_langs
        .iter()
        .map(|lang| format!("/{lang}/").to_lowercase())
        .collect();

    let original_len = assets.len();
    assets.retain(|asset| {
        let asset_lower = asset.asset_name.to_lowercase();

        for pattern in &patterns_lower {
            if asset_lower.contains(pattern) {
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
    let persistent_dir = super::find_genshin_persistent_dir(game_dir);
    fs::create_dir_all(&persistent_dir)?;

    write_lang_file(
        &persistent_dir.join(AUDIO_LANG_FILE),
        vo_langs,
        locale_code_to_audio_lang_name,
    )
}

fn locale_code_to_audio_lang_name(locale: &str) -> Option<&'static str> {
    match locale {
        "zh-cn" | "cn" | "zh-tw" => Some("Chinese"),
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
    assets: &Arc<Vec<SophonManifestAssetProperty>>,
    vo_langs: &[String],
) -> std::io::Result<()> {
    write_single_pkg_version(game_dir, "pkg_version", assets)?;

    for lang in vo_langs {
        if let Some(lang_name) = locale_code_to_audio_lang_name(lang) {
            let filename = format!("Audio_{lang_name}_pkg_version");
            let pattern = format!("/{lang_name}/").to_lowercase();
            let filtered: Vec<SophonManifestAssetProperty> = assets
                .iter()
                .filter(|a| a.asset_name.to_lowercase().contains(&pattern))
                .cloned()
                .collect();
            write_single_pkg_version(game_dir, &filename, &filtered)?;
        }
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // -----------------------------------------------------------------------
    // locale_code_to_audio_lang_name (tested via super::)
    // -----------------------------------------------------------------------
    #[test]
    fn test_locale_code_to_audio_lang_name_zh_cn() {
        assert_eq!(locale_code_to_audio_lang_name("zh-cn"), Some("Chinese"));
    }

    #[test]
    fn test_locale_code_to_audio_lang_name_cn() {
        assert_eq!(locale_code_to_audio_lang_name("cn"), Some("Chinese"));
    }

    #[test]
    fn test_locale_code_to_audio_lang_name_en_us() {
        assert_eq!(locale_code_to_audio_lang_name("en-us"), Some("English(US)"));
    }

    #[test]
    fn test_locale_code_to_audio_lang_name_en() {
        assert_eq!(locale_code_to_audio_lang_name("en"), Some("English(US)"));
    }

    #[test]
    fn test_locale_code_to_audio_lang_name_ja_jp() {
        assert_eq!(locale_code_to_audio_lang_name("ja-jp"), Some("Japanese"));
    }

    #[test]
    fn test_locale_code_to_audio_lang_name_jp() {
        assert_eq!(locale_code_to_audio_lang_name("jp"), Some("Japanese"));
    }

    #[test]
    fn test_locale_code_to_audio_lang_name_ko_kr() {
        assert_eq!(locale_code_to_audio_lang_name("ko-kr"), Some("Korean"));
    }

    #[test]
    fn test_locale_code_to_audio_lang_name_kr() {
        assert_eq!(locale_code_to_audio_lang_name("kr"), Some("Korean"));
    }

    #[test]
    fn test_locale_code_to_audio_lang_name_unknown() {
        assert_eq!(locale_code_to_audio_lang_name("fr-fr"), None);
        assert_eq!(locale_code_to_audio_lang_name(""), None);
    }

    // -----------------------------------------------------------------------
    // filter_hk4e_asset_list
    // -----------------------------------------------------------------------
    #[test]
    fn test_filter_hk4e_asset_list_filters_uninstalled_languages() {
        let dir = tempfile::tempdir().unwrap();
        let persistent_dir = dir.path().join("GenshinImpact_Data/Persistent");
        fs::create_dir_all(&persistent_dir).unwrap();
        // Chinese and English(US) are installed
        fs::write(
            persistent_dir.join("audio_lang_14"),
            "Chinese\nEnglish(US)\n",
        )
        .unwrap();

        let mut assets = vec![
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/Chinese/file.pck".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/English(US)/file.pck".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/Japanese/file.pck".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/Korean/file.pck".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/non_audio/file.dat".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
        ];

        filter_hk4e_asset_list(
            dir.path(),
            &mut assets,
            &["en-us".to_string(), "zh-cn".to_string()],
        );

        // Chinese and English(US) should be kept, Japanese and Korean filtered
        assert_eq!(assets.len(), 3);
        assert!(assets.iter().any(|a| a.asset_name.contains("Chinese")));
        assert!(assets.iter().any(|a| a.asset_name.contains("English(US)")));
        assert!(assets.iter().any(|a| a.asset_name.contains("non_audio")));
        assert!(!assets.iter().any(|a| a.asset_name.contains("Japanese")));
        assert!(!assets.iter().any(|a| a.asset_name.contains("Korean")));
    }

    #[test]
    fn test_filter_hk4e_asset_list_filters_ctable_files() {
        let dir = tempfile::tempdir().unwrap();
        let persistent_dir = dir.path().join("GenshinImpact_Data/Persistent");
        fs::create_dir_all(&persistent_dir).unwrap();
        fs::write(persistent_dir.join("audio_lang_14"), "Chinese\n").unwrap();

        let mut assets = vec![
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/Chinese/ctable_streaming.dat".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/normal_file.pck".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
        ];

        filter_hk4e_asset_list(dir.path(), &mut assets, &["zh-cn".to_string()]);

        // ctable file should be filtered even though Chinese is installed
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_name, "data/asset_bundle/normal_file.pck");
    }

    #[test]
    fn test_filter_hk4e_asset_list_all_languages_installed() {
        let dir = tempfile::tempdir().unwrap();
        let persistent_dir = dir.path().join("GenshinImpact_Data/Persistent");
        fs::create_dir_all(&persistent_dir).unwrap();
        fs::write(
            persistent_dir.join("audio_lang_14"),
            "Chinese\nEnglish(US)\nJapanese\nKorean\n",
        )
        .unwrap();

        let mut assets = vec![
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/Chinese/file.pck".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/Japanese/file.pck".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
        ];

        filter_hk4e_asset_list(
            dir.path(),
            &mut assets,
            &[
                "zh-cn".to_string(),
                "en-us".to_string(),
                "ja-jp".to_string(),
                "ko-kr".to_string(),
            ],
        );

        assert_eq!(assets.len(), 2);
    }

    #[test]
    fn test_filter_hk4e_asset_list_empty_vo_langs_filters_all_audio() {
        let dir = tempfile::tempdir().unwrap();
        let persistent_dir = dir.path().join("GenshinImpact_Data/Persistent");
        fs::create_dir_all(&persistent_dir).unwrap();
        // No audio_lang_14 file -> falls back to vo_langs mapping which is empty
        // So no installed_langs -> all audio languages are ignored

        let mut assets = vec![
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/Chinese/file.pck".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/non_audio/file.dat".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "abc".into(),
            },
        ];

        filter_hk4e_asset_list(dir.path(), &mut assets, &[]);

        // With empty vo_langs, all audio is filtered, only non-audio remains
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_name, "data/asset_bundle/non_audio/file.dat");
    }

    // -----------------------------------------------------------------------
    // write_audio_lang_record
    // -----------------------------------------------------------------------
    #[test]
    fn test_write_audio_lang_record_creates_persistent_dir_and_file() {
        let dir = tempfile::tempdir().unwrap();

        let vo_langs = vec!["zh-cn".to_string(), "en-us".to_string()];
        write_audio_lang_record(dir.path(), &vo_langs).unwrap();

        let persistent_dir = dir.path().join("GenshinImpact_Data/Persistent");
        assert!(persistent_dir.exists());

        let content = fs::read_to_string(persistent_dir.join("audio_lang_14")).unwrap();
        assert_eq!(content, "Chinese\nEnglish(US)\n");
    }

    // -----------------------------------------------------------------------
    // write_pkg_version_from_manifest
    // -----------------------------------------------------------------------
    #[test]
    fn test_write_pkg_version_from_manifest_creates_files() {
        let dir = tempfile::tempdir().unwrap();

        let assets = Arc::new(vec![
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/Chinese/file.pck".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 100,
                asset_hash_md5: "md5_chinese".into(),
            },
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/English(US)/file.pck".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 200,
                asset_hash_md5: "md5_english".into(),
            },
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/common/data.bin".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 300,
                asset_hash_md5: "md5_common".into(),
            },
            // Directory entry - should be skipped
            SophonManifestAssetProperty {
                asset_name: "data/asset_bundle/".into(),
                asset_chunks: vec![],
                asset_type: 64,
                asset_size: 0,
                asset_hash_md5: String::new(),
            },
        ]);

        let vo_langs = vec!["zh-cn".to_string(), "en-us".to_string()];
        write_pkg_version_from_manifest(dir.path(), &assets, &vo_langs).unwrap();

        // Check pkg_version contains all non-directory assets
        let pkg_content = fs::read_to_string(dir.path().join("pkg_version")).unwrap();
        let lines: Vec<&str> = pkg_content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 3);

        // Check Audio_Chinese_pkg_version
        let audio_cn = fs::read_to_string(dir.path().join("Audio_Chinese_pkg_version")).unwrap();
        let cn_lines: Vec<&str> = audio_cn.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(cn_lines.len(), 1);
        assert!(cn_lines[0].contains("Chinese"));

        // Check Audio_English(US)_pkg_version
        let audio_en =
            fs::read_to_string(dir.path().join("Audio_English(US)_pkg_version")).unwrap();
        let en_lines: Vec<&str> = audio_en.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(en_lines.len(), 1);
        assert!(en_lines[0].contains("English(US)"));
    }

    #[test]
    fn test_write_pkg_version_from_manifest_no_audio_langs() {
        let dir = tempfile::tempdir().unwrap();

        let assets = Arc::new(vec![SophonManifestAssetProperty {
            asset_name: "data/common.bin".into(),
            asset_chunks: vec![],
            asset_type: 0,
            asset_size: 100,
            asset_hash_md5: "md5".into(),
        }]);

        write_pkg_version_from_manifest(dir.path(), &assets, &[]).unwrap();

        // pkg_version should exist
        assert!(dir.path().join("pkg_version").exists());
        // No audio-specific files
        assert!(!dir.path().join("Audio_Chinese_pkg_version").exists());
    }

    // -----------------------------------------------------------------------
    // read_installed_audio_langs (private fn tested via super::)
    // -----------------------------------------------------------------------
    #[test]
    fn test_read_installed_audio_langs_from_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("GenshinImpact_Data/Persistent")).unwrap();
        fs::write(
            dir.path()
                .join("GenshinImpact_Data/Persistent/audio_lang_14"),
            "Chinese\nEnglish(US)\n",
        )
        .unwrap();

        let persistent_dir = super::super::find_genshin_persistent_dir(dir.path());
        let result = read_installed_audio_langs(&persistent_dir, &["ja-jp".to_string()]);
        // Should read from the file, not use vo_langs fallback
        assert_eq!(
            result,
            vec!["Chinese".to_string(), "English(US)".to_string()]
        );
    }

    #[test]
    fn test_read_installed_audio_langs_fallback_to_vo_langs() {
        let dir = tempfile::tempdir().unwrap();
        let persistent_dir = dir.path().join("GenshinImpact_Data/Persistent");
        fs::create_dir_all(&persistent_dir).unwrap();
        // No audio_lang_* files

        let result = read_installed_audio_langs(
            &persistent_dir,
            &["ja-jp".to_string(), "ko-kr".to_string()],
        );
        // Falls back to vo_langs mapped names
        assert_eq!(result, vec!["Japanese".to_string(), "Korean".to_string()]);
    }

    #[test]
    fn test_read_installed_audio_langs_empty_vo_langs_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let persistent_dir = dir.path().join("GenshinImpact_Data/Persistent");
        fs::create_dir_all(&persistent_dir).unwrap();

        let result = read_installed_audio_langs(&persistent_dir, &[]);
        assert!(result.is_empty());
    }

    // -----------------------------------------------------------------------
    // write_single_pkg_version (private fn tested via super::)
    // -----------------------------------------------------------------------
    #[test]
    fn test_write_single_pkg_version_skips_directories() {
        let dir = tempfile::tempdir().unwrap();
        let assets = vec![
            SophonManifestAssetProperty {
                asset_name: "dir/".into(),
                asset_chunks: vec![],
                asset_type: 64,
                asset_size: 0,
                asset_hash_md5: String::new(),
            },
            SophonManifestAssetProperty {
                asset_name: "data/file.bin".into(),
                asset_chunks: vec![],
                asset_type: 0,
                asset_size: 50,
                asset_hash_md5: "hash".into(),
            },
        ];

        write_single_pkg_version(dir.path(), "test_pkg_version", &assets).unwrap();
        let content = fs::read_to_string(dir.path().join("test_pkg_version")).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("data/file.bin"));
    }
}
