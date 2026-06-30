use std::collections::{HashMap, HashSet};
use std::path::Path;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tauri_plugin_log::log;

use super::api::{fetch_build, is_known_vo_locale, vo_lang_matches};
use super::error::{SophonError, SophonResult};
use super::read_installed_tag;

use crate::commands::sophon_downloader::api_scrape::PackageBranch;
use crate::commands::sophon_downloader::api_scrape::SophonManifestMeta;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub update_available: bool,
    pub preinstall_available: bool,
    pub preinstall_downloaded: bool,
    pub current_tag: Option<String>,
    pub remote_tag: String,
    pub preinstall_tag: Option<String>,
    pub update_compressed_size: u64,
    pub update_decompressed_size: u64,
    pub preinstall_compressed_size: u64,
    pub preinstall_decompressed_size: u64,
}

pub async fn check_update(
    client: &Client,
    game_id: &str,
    vo_lang: &str,
    game_dir: &Path,
) -> SophonResult<UpdateInfo> {
    let (front_door, current_tag) =
        tokio::join!(super::api::fetch_front_door(client, game_id), async {
            read_installed_tag(game_dir)
        });
    let (branch, pre_download_branch) = front_door?;

    let main_branch = branch.main.as_ref().ok_or(SophonError::NoGameManifest)?;
    let remote_tag = main_branch.tag.clone();

    let update_available = current_tag
        .as_deref()
        .map(|t| t != remote_tag)
        .unwrap_or(false);

    let (update_compressed_size, update_decompressed_size) = if update_available {
        if let Some(ref installed) = current_tag {
            fetch_diff_sizes(client, main_branch, installed, &remote_tag, vo_lang).await?
        } else {
            (0, 0)
        }
    } else {
        (0, 0)
    };

    let (
        preinstall_available,
        mut preinstall_tag,
        preinstall_compressed_size,
        preinstall_decompressed_size,
    ) = match pre_download_branch {
        Some(ref pre) => {
            let tag = pre.tag.clone();
            match fetch_build_sizes(client, pre, vo_lang).await {
                Ok((cs, ds)) => (true, Some(tag), cs, ds),
                Err(err) => {
                    log::warn!("Failed to fetch preinstall sizes: {err}");
                    (true, Some(tag), 0, 0)
                }
            }
        }
        None => (false, None, 0, 0),
    };

    let preinstall_downloaded = if let Some(ref ptag) = preinstall_tag {
        let marker = game_dir.join(format!(".sophon_preinstall_{ptag}"));
        let state_file = game_dir.join(format!(".sophon_preinstall_{ptag}.json"));
        marker.exists() || state_file.exists()
    } else if update_available {
        let marker = game_dir.join(format!(".sophon_preinstall_{remote_tag}"));
        let state_file = game_dir.join(format!(".sophon_preinstall_{remote_tag}.json"));
        let downloaded = marker.exists() || state_file.exists();
        if downloaded {
            preinstall_tag = Some(remote_tag.clone());
        }
        downloaded
    } else {
        false
    };

    Ok(UpdateInfo {
        update_available,
        preinstall_available,
        preinstall_downloaded,
        current_tag,
        remote_tag,
        preinstall_tag,
        update_compressed_size,
        update_decompressed_size,
        preinstall_compressed_size,
        preinstall_decompressed_size,
    })
}

pub async fn fetch_build_sizes(
    client: &Client,
    branch: &PackageBranch,
    vo_lang: &str,
) -> SophonResult<(u64, u64)> {
    let build = fetch_build(client, branch, None).await?;

    // Match the same filter as build_installers_from_data:
    // game + VO language + non-VO manifests (cutscenes, etc.)
    let qualifying: Vec<&SophonManifestMeta> = build
        .manifests
        .iter()
        .filter(|m| {
            m.matching_field == "game"
                || vo_lang_matches(&m.matching_field, vo_lang)
                || !is_known_vo_locale(&m.matching_field)
        })
        .collect();

    if qualifying.is_empty() {
        return Err(SophonError::NoGameManifest);
    }

    let mut cs = 0u64;
    let mut ds = 0u64;
    for meta in qualifying {
        cs += super::api::parse_size(&meta.stats.compressed_size)?;
        ds += super::api::parse_size(&meta.stats.uncompressed_size)?;
    }
    Ok((cs, ds))
}

pub async fn fetch_diff_sizes(
    client: &Client,
    branch: &PackageBranch,
    from_tag: &str,
    to_tag: &str,
    vo_lang: &str,
) -> SophonResult<(u64, u64)> {
    let (old_build, new_build) = tokio::try_join!(
        fetch_build(client, branch, Some(from_tag)),
        fetch_build(client, branch, Some(to_tag)),
    )?;

    let mut cs = 0u64;
    let mut ds = 0u64;
    let mut seen_chunks: HashSet<String> = HashSet::new();

    let old_map: HashMap<String, &SophonManifestMeta> = old_build
        .manifests
        .iter()
        .map(|m| (m.matching_field.clone(), m))
        .collect();

    for new_meta in &new_build.manifests {
        if new_meta.matching_field != "game"
            && !vo_lang_matches(&new_meta.matching_field, vo_lang)
            && is_known_vo_locale(&new_meta.matching_field)
        {
            continue;
        }

        let matching_field = new_meta.matching_field.clone();

        let (new_response, old_data) = tokio::try_join!(
            super::api::fetch_manifest(client, &new_meta.manifest_download, &new_meta.manifest.id),
            async {
                match old_map.get(&matching_field) {
                    Some(old_meta) => {
                        let old_manifest = super::api::fetch_manifest(
                            client,
                            &old_meta.manifest_download,
                            &old_meta.manifest.id,
                        )
                        .await?
                        .manifest;
                        // Build both file-MD5 map and chunk-decompressed-hash set
                        let old_files_md5 = old_manifest
                            .assets
                            .iter()
                            .filter(|f| !f.is_directory())
                            .map(|f| (f.asset_name.clone(), f.asset_hash_md5.clone()))
                            .collect::<HashMap<String, String>>();
                        let old_chunks: HashSet<String> = old_manifest
                            .assets
                            .iter()
                            .filter(|f| !f.is_directory())
                            .flat_map(|f| f.asset_chunks.iter())
                            .map(|c| c.chunk_decompressed_hash_md5.clone())
                            .collect();
                        Ok(Some((old_files_md5, old_chunks)))
                    }
                    None => {
                        Ok::<Option<(HashMap<String, String>, HashSet<String>)>, SophonError>(None)
                    }
                }
            }
        )?;
        let new_manifest = new_response.manifest;

        let (old_files_md5, old_chunks): (HashMap<String, String>, HashSet<String>) = match old_data
        {
            Some((md5, chunks)) => (md5, chunks),
            None => (HashMap::new(), HashSet::new()),
        };

        for file in &new_manifest.assets {
            if file.is_directory() {
                continue;
            }
            let needs_download = match old_files_md5.get(&file.asset_name) {
                Some(old_md5) => old_md5 != &file.asset_hash_md5,
                None => true,
            };
            if needs_download {
                for chunk in &file.asset_chunks {
                    // Skip chunks whose decompressed content already exists in an old file
                    if old_chunks.contains(&chunk.chunk_decompressed_hash_md5) {
                        continue;
                    }
                    if seen_chunks.insert(chunk.chunk_name.clone()) {
                        cs += chunk.chunk_size;
                        ds += chunk.chunk_size_decompressed;
                    }
                }
            }
        }
    }

    Ok((cs, ds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::sophon_downloader::game_installer::api;

    #[test]
    fn parse_size_returns_correct_values() {
        assert_eq!(api::parse_size("0").unwrap(), 0);
        assert_eq!(api::parse_size("1024").unwrap(), 1024);
        assert_eq!(api::parse_size("2048").unwrap(), 2048);
        assert_eq!(api::parse_size("999999").unwrap(), 999999);
    }

    #[test]
    fn is_known_vo_locale_identifies_known_and_unknown() {
        assert!(is_known_vo_locale("en-us"));
        assert!(is_known_vo_locale("ja-jp"));
        assert!(is_known_vo_locale("zh-cn"));
        assert!(!is_known_vo_locale("game"));
        assert!(!is_known_vo_locale("cutscenes"));
        assert!(!is_known_vo_locale(""));
    }
    #[test]
    fn vo_lang_matches_cases() {
        // CN matches zh-cn
        assert!(vo_lang_matches("zh-cn", "cn"));
        assert!(vo_lang_matches("zh-tw", "cn"));
        // EN matches en-us
        assert!(vo_lang_matches("en-us", "en"));
        // JP matches ja-jp
        assert!(vo_lang_matches("ja-jp", "jp"));
        // KR matches ko-kr
        assert!(vo_lang_matches("ko-kr", "kr"));
        // Wrong language doesn't match
        assert!(!vo_lang_matches("en-us", "jp"));
        assert!(!vo_lang_matches("ja-jp", "en"));
        // Case insensitive
        assert!(vo_lang_matches("EN-US", "en"));
        assert!(vo_lang_matches("zh-cn", "CN"));
        // Empty doesn't match
        assert!(!vo_lang_matches("", ""));
        assert!(!vo_lang_matches("game", "en"));
    }

    #[test]
    fn update_info_serde_roundtrip_basic() {
        let info = UpdateInfo {
            update_available: true,
            preinstall_available: false,
            preinstall_downloaded: false,
            current_tag: Some("1.0.0".into()),
            remote_tag: "2.0.0".into(),
            preinstall_tag: None,
            update_compressed_size: 1_000_000,
            update_decompressed_size: 5_000_000,
            preinstall_compressed_size: 0,
            preinstall_decompressed_size: 0,
        };
        let json = serde_json::to_string(&info).unwrap();
        let decoded: UpdateInfo = serde_json::from_str(&json).unwrap();
        assert!(decoded.update_available);
        assert!(!decoded.preinstall_available);
        assert_eq!(decoded.current_tag, Some("1.0.0".to_string()));
        assert_eq!(decoded.remote_tag, "2.0.0");
        assert_eq!(decoded.update_compressed_size, 1_000_000);
    }

    #[test]
    fn update_info_serde_roundtrip_all_fields() {
        let info = UpdateInfo {
            update_available: false,
            preinstall_available: true,
            preinstall_downloaded: true,
            current_tag: None,
            remote_tag: "3.0.0-pre".into(),
            preinstall_tag: Some("3.0.0-pre".into()),
            update_compressed_size: u64::MAX,
            update_decompressed_size: u64::MAX,
            preinstall_compressed_size: 0,
            preinstall_decompressed_size: 0,
        };
        let json = serde_json::to_string(&info).unwrap();
        let decoded: UpdateInfo = serde_json::from_str(&json).unwrap();
        assert!(decoded.preinstall_available);
        assert_eq!(decoded.current_tag, None);
        assert_eq!(decoded.preinstall_tag, Some("3.0.0-pre".to_string()));
        assert_eq!(decoded.update_compressed_size, u64::MAX);
    }

    #[test]
    fn update_info_deserialize_missing_field_fails() {
        let json = r#"{"update_available":true}"#;
        let result: Result<UpdateInfo, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }
    #[test]
    fn update_info_no_update_when_tags_match() {
        let info = UpdateInfo {
            update_available: false,
            preinstall_available: false,
            preinstall_downloaded: false,
            current_tag: Some("1.0.0".into()),
            remote_tag: "1.0.0".into(),
            preinstall_tag: None,
            update_compressed_size: 0,
            update_decompressed_size: 0,
            preinstall_compressed_size: 0,
            preinstall_decompressed_size: 0,
        };
        assert!(!info.update_available);
        assert_eq!(info.current_tag.as_deref(), Some("1.0.0"));
        assert_eq!(info.current_tag.as_deref(), Some(info.remote_tag.as_str()));
    }
}
