use std::collections::HashMap;
use std::path::Path;

use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::api::{fetch_build, vo_lang_matches};
use super::version::read_installed_tag;
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
) -> Result<UpdateInfo, Box<dyn std::error::Error + Send + Sync>> {
    let (front_door, current_tag) =
        tokio::join!(super::api::fetch_front_door(client, game_id), async {
            read_installed_tag(game_dir)
        });
    let (branch, pre_download_branch) = front_door?;

    let remote_tag = branch.main.tag.clone();

    let update_available = current_tag
        .as_deref()
        .map(|t| t != remote_tag)
        .unwrap_or(false);

    let (update_compressed_size, update_decompressed_size) = if update_available {
        if let Some(ref installed) = current_tag {
            match fetch_diff_sizes(client, &branch.main, installed, &remote_tag, vo_lang).await {
                Ok(sizes) => sizes,
                Err(_) => (0, 0),
            }
        } else {
            (0, 0)
        }
    } else {
        (0, 0)
    };

    let (
        preinstall_available,
        preinstall_tag,
        preinstall_compressed_size,
        preinstall_decompressed_size,
    ) = match pre_download_branch {
        Some(ref pre) => {
            let tag = pre.tag.clone();
            let (cs, ds) = fetch_build_sizes(client, pre, vo_lang)
                .await
                .unwrap_or((0, 0));
            (true, Some(tag), cs, ds)
        }
        None => (false, None, 0, 0),
    };

    let preinstall_downloaded = if let Some(ref ptag) = preinstall_tag {
        game_dir.join(format!(".sophon_preinstall_{ptag}")).exists()
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
) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
    let build = fetch_build(client, branch, None).await?;
    let game_meta = build.manifests.first().ok_or("no manifests")?;
    let vo_meta = build
        .manifests
        .iter()
        .find(|m| vo_lang_matches(&m.matching_field, vo_lang))
        .ok_or("No VO manifest matching language")?;

    let cs = super::api::parse_size(&game_meta.stats.compressed_size)
        + super::api::parse_size(&vo_meta.stats.compressed_size);
    let ds = super::api::parse_size(&game_meta.stats.uncompressed_size)
        + super::api::parse_size(&vo_meta.stats.uncompressed_size);
    Ok((cs, ds))
}

pub async fn fetch_diff_sizes(
    client: &Client,
    branch: &PackageBranch,
    from_tag: &str,
    to_tag: &str,
    vo_lang: &str,
) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
    let (old_build, new_build) = tokio::try_join!(
        fetch_build(client, branch, Some(from_tag)),
        fetch_build(client, branch, Some(to_tag)),
    )?;

    let mut cs = 0u64;
    let mut ds = 0u64;

    let old_map: HashMap<String, &SophonManifestMeta> = old_build
        .manifests
        .iter()
        .map(|m| (m.matching_field.clone(), m))
        .collect();

    for new_meta in &new_build.manifests {
        if new_meta.matching_field != "game" && !vo_lang_matches(&new_meta.matching_field, vo_lang)
        {
            continue;
        }

        let new_manifest =
            super::api::fetch_manifest(client, &new_meta.manifest_download, &new_meta.manifest.id)
                .await?;

        let old_files: HashMap<String, String> = match old_map.get(&new_meta.matching_field) {
            Some(old_meta) => {
                let old_manifest = super::api::fetch_manifest(
                    client,
                    &old_meta.manifest_download,
                    &old_meta.manifest.id,
                )
                .await?;
                old_manifest
                    .assets
                    .into_iter()
                    .filter(|f| !f.is_directory())
                    .map(|f| (f.asset_name, f.asset_hash_md5))
                    .collect()
            }
            None => HashMap::new(),
        };

        for file in &new_manifest.assets {
            if file.is_directory() {
                continue;
            }
            let needs_download = match old_files.get(&file.asset_name) {
                Some(old_md5) => old_md5 != &file.asset_hash_md5,
                None => true,
            };
            if needs_download {
                for chunk in &file.asset_chunks {
                    cs += chunk.chunk_size;
                    ds += chunk.chunk_size_decompressed;
                }
            }
        }
    }

    Ok((cs, ds))
}
