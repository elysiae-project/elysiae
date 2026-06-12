mod hk4e;
mod hkrpg;
mod nap;

pub use hk4e::{
    filter_hk4e_asset_list, write_audio_lang_record as write_hk4e_audio_lang_record,
    write_pkg_version_from_manifest,
};
pub use hkrpg::{
    filter_hkrpg_asset_list, write_app_info as write_hkrpg_app_info,
    write_audio_lang_record as write_hkrpg_audio_lang_record,
    write_binary_version_files as write_hkrpg_binary_version_files,
};
pub use nap::{filter_nap_asset_list, filter_nap_installers, write_nap_audio_lang_records};

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

pub(crate) fn find_genshin_persistent_dir(game_dir: &Path) -> std::path::PathBuf {
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

pub(crate) fn write_lang_file(
    path: &Path,
    vo_langs: &[String],
    mapper: fn(&str) -> Option<&'static str>,
) -> std::io::Result<()> {
    let mut existing: Vec<String> = Vec::new();
    if path.exists()
        && let Ok(content) = fs::read_to_string(path)
    {
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                existing.push(trimmed.to_string());
            }
        }
    }

    for lang in vo_langs {
        if let Some(name) = mapper(lang)
            && !existing.iter().any(|e| e == name)
        {
            existing.push(name.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // -----------------------------------------------------------------------
    // find_genshin_persistent_dir
    // -----------------------------------------------------------------------
    #[test]
    fn test_find_genshin_persistent_dir_with_genshin_data() {
        let dir = tempfile::tempdir().unwrap();
        let genshin_data = dir.path().join("GenshinImpact_Data");
        fs::create_dir(&genshin_data).unwrap();

        let result = find_genshin_persistent_dir(dir.path());
        assert_eq!(result, genshin_data.join("Persistent"));
    }

    #[test]
    fn test_find_genshin_persistent_dir_with_yuanshen_data() {
        let dir = tempfile::tempdir().unwrap();
        let yuanshen_data = dir.path().join("YuanShen_Data");
        fs::create_dir(&yuanshen_data).unwrap();

        let result = find_genshin_persistent_dir(dir.path());
        assert_eq!(result, yuanshen_data.join("Persistent"));
    }

    #[test]
    fn test_find_genshin_persistent_dir_with_neither() {
        let dir = tempfile::tempdir().unwrap();
        // Create a random dir that doesn't match
        let other_data = dir.path().join("Other_Data");
        fs::create_dir(&other_data).unwrap();

        let result = find_genshin_persistent_dir(dir.path());
        assert_eq!(result, dir.path().join("GenshinImpact_Data/Persistent"));
    }

    #[test]
    fn test_find_genshin_persistent_dir_empty_directory() {
        let dir = tempfile::tempdir().unwrap();

        let result = find_genshin_persistent_dir(dir.path());
        assert_eq!(result, dir.path().join("GenshinImpact_Data/Persistent"));
    }

    // -----------------------------------------------------------------------
    // write_lang_file
    // -----------------------------------------------------------------------
    #[test]
    fn test_write_lang_file_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lang_file.txt");

        let vo_langs = vec!["en-us".to_string(), "ja-jp".to_string()];

        write_lang_file(&path, &vo_langs, |locale| match locale {
            "en-us" => Some("English(US)"),
            "ja-jp" => Some("Japanese"),
            _ => None,
        })
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "English(US)\nJapanese\n");
    }

    #[test]
    fn test_write_lang_file_append_to_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lang_file.txt");
        fs::write(&path, "Chinese\n").unwrap();

        let vo_langs = vec!["en-us".to_string(), "ja-jp".to_string()];

        write_lang_file(&path, &vo_langs, |locale| match locale {
            "en-us" => Some("English(US)"),
            "ja-jp" => Some("Japanese"),
            _ => None,
        })
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "Chinese\nEnglish(US)\nJapanese\n");
    }

    #[test]
    fn test_write_lang_file_does_not_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lang_file.txt");
        fs::write(&path, "English(US)\n").unwrap();

        let vo_langs = vec!["en-us".to_string()];

        write_lang_file(&path, &vo_langs, |locale| match locale {
            "en-us" => Some("English(US)"),
            _ => None,
        })
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "English(US)\n");
    }

    #[test]
    fn test_write_lang_file_empty_vo_langs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lang_file.txt");

        write_lang_file(&path, &[], |_| -> Option<&'static str> { None }).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn test_write_lang_file_skips_none_results() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lang_file.txt");

        let vo_langs = vec![
            "en-us".to_string(),
            "unknown".to_string(),
            "ja-jp".to_string(),
        ];

        write_lang_file(&path, &vo_langs, |locale| match locale {
            "en-us" => Some("English(US)"),
            "ja-jp" => Some("Japanese"),
            "unknown" => None,
            _ => None,
        })
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "English(US)\nJapanese\n");
    }

    #[test]
    fn test_write_lang_file_mapper_en_us() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lang_file.txt");

        let vo_langs = vec!["en-us".to_string()];

        write_lang_file(&path, &vo_langs, |locale| match locale {
            "en-us" => Some("English(US)"),
            _ => None,
        })
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "English(US)\n");
    }
}
