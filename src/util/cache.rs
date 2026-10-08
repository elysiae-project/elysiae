use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{
    core::{
        fs::{full_path, read_dir, remove, write_file},
        game::Game,
    },
    util::web::{download_file, fetch_data},
};

#[derive(Clone, Copy)]
pub enum AssetType {
    Image,
    Video,
    Icon,
    Shortcut,
    Overlay,
}

#[derive(Debug, Serialize, Deserialize)]
struct AedesResponse {
    backgrounds: Vec<AedesBackgroundAssets>,
    icon: String,
    icon_cn: String,
    shortcut: String,
    shortcut_cn: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AedesBackgroundAssets {
    image: String,
    video: Option<String>,
    overlay: Option<String>,
}

pub async fn update_cache() -> Result<()> {
    let games = [
        Game::Bh3,
        Game::Hk4e,
        Game::Hkrpg,
        Game::Nap,
        Game::Abc,
        Game::Hyg,
    ];
    let locale = "en-us"; // TODO: Replace with proper locale calls later

    let mut g = futures_util::stream::iter(games);
    while let Some(game) = g.next().await {
        let mut url = url::Url::parse("https://aedes.elysiae.app/getAssets")?;
        url.query_pairs_mut()
            .append_pair("lang", locale)
            .append_pair("game", game.code());
        let response = fetch_data::<AedesResponse>(url.as_str()).await?;

        let desired_paths = response
            .asset_paths()
            .filter_map(|path| path.map(PathBuf::from))
            .collect::<Vec<_>>();
        let cache_dir = PathBuf::from(format!("cache/{}/{locale}", game.code()));
        let files_present = if cache_dir.try_exists()? {
            read_dir(cache_dir, None)?
        } else {
            Vec::new()
        };

        for path in &desired_paths {
            let destination = cache_path(path)?;
            if !destination.try_exists()? {
                let url = format!("https://aedes.elysiae.app{}", path.display());
                download_file(url, destination, None, None).await?;
            }
        }

        for file in files_present {
            if file.file_name().is_some_and(|name| name == "assets.json") {
                continue;
            }
            if !desired_paths
                .iter()
                .filter_map(|path| cache_path(path).ok())
                .any(|path| path == file)
            {
                remove(file, None, Some(true))?;
            }
        }

        write_file(
            metadata_path(game, locale),
            &serde_json::to_vec_pretty(&response)?,
            None,
        )?;
    }
    Ok(())
}

pub fn cache_available(locale: &str) -> Result<bool> {
    for game in [Game::Bh3, Game::Hk4e, Game::Hkrpg, Game::Nap, Game::Abc, Game::Hyg] {
        let metadata = full_path(Some(metadata_path(game, locale)), None)?;
        if !metadata.is_file() {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn media_paths(game: Game, locale: &str, use_video: bool) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    let primary_type = if use_video { AssetType::Video } else { AssetType::Image };
    let primary = get_cached_asset_paths(game, locale, primary_type)?.into_iter().next();
    let fallback = if primary.is_none() {
        get_cached_asset_paths(game, locale, AssetType::Image)?.into_iter().next()
    } else {
        None
    };
    let overlay = get_cached_asset_paths(game, locale, AssetType::Overlay)?.into_iter().next();
    Ok((primary.or(fallback), overlay))
}

impl AedesResponse {
    fn asset_paths(&self) -> impl Iterator<Item = Option<&str>> {
        self.backgrounds
            .iter()
            .flat_map(|background| {
                [
                    Some(background.image.as_str()),
                    background.video.as_deref(),
                    background.overlay.as_deref(),
                ]
            })
            .chain([Some(self.icon.as_str()), Some(self.shortcut.as_str())])
    }
}

fn metadata_path(game: Game, locale: &str) -> PathBuf {
    PathBuf::from(format!("cache/{}/{locale}/assets.json", game.code()))
}

fn cache_path(path: &PathBuf) -> Result<PathBuf> {
    let path = path
        .strip_prefix("/")
        .context("Asset path must be absolute on the Aedes server")?;
    ensure!(
        path.components()
            .all(|component| matches!(component, std::path::Component::Normal(_))),
        "Asset path contains unsafe components"
    );
    Ok(PathBuf::from("cache").join(path))
}

pub fn get_cached_asset_paths(
    game: Game,
    locale: &str,
    asset_type: AssetType,
) -> Result<Vec<PathBuf>> {
    let metadata = full_path(Some(metadata_path(game, locale)), None)?;
    ensure!(metadata.try_exists()?, "Asset metadata is not cached");
    let response = serde_json::from_slice::<AedesResponse>(&std::fs::read(metadata)?)?;
    let paths = match asset_type {
        AssetType::Image => response
            .backgrounds
            .iter()
            .map(|asset| Some(asset.image.as_str()))
            .collect(),
        AssetType::Video => response
            .backgrounds
            .iter()
            .map(|asset| asset.video.as_deref())
            .collect(),
        AssetType::Overlay => response
            .backgrounds
            .iter()
            .map(|asset| asset.overlay.as_deref())
            .collect(),
        AssetType::Icon => vec![Some(response.icon.as_str())],
        AssetType::Shortcut => vec![Some(response.shortcut.as_str())],
    };

    paths
        .into_iter()
        .flatten()
        .map(|path| {
            let cached = cache_path(&PathBuf::from(path))?;
            let full = full_path(Some(cached), None)?;
            ensure!(
                full.is_file(),
                "Cached asset is missing: {}",
                full.display()
            );
            Ok(full)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_paths_preserve_order_and_skip_optional_assets() {
        let response = AedesResponse {
            backgrounds: vec![AedesBackgroundAssets {
                image: "/images/a.png".into(),
                video: Some("/videos/a.mp4".into()),
                overlay: None,
            }],
            icon: "/icons/a.png".into(),
            icon_cn: "/icons/a-cn.png".into(),
            shortcut: "/shortcuts/a.png".into(),
            shortcut_cn: "/shortcuts/a-cn.png".into(),
        };
        let paths: Vec<_> = response.asset_paths().flatten().collect();
        assert_eq!(
            paths,
            vec![
                "/images/a.png",
                "/videos/a.mp4",
                "/icons/a.png",
                "/shortcuts/a.png"
            ]
        );
    }

    #[test]
    fn cache_paths_require_safe_absolute_server_paths() {
        assert_eq!(
            cache_path(&PathBuf::from("/assets/a.png")).unwrap(),
            PathBuf::from("cache/assets/a.png")
        );
        assert!(cache_path(&PathBuf::from("assets/a.png")).is_err());
        assert!(cache_path(&PathBuf::from("/../secret")).is_err());
    }

    #[test]
    fn metadata_paths_use_game_and_locale() {
        assert_eq!(
            metadata_path(Game::Bh3, "en-us"),
            PathBuf::from("cache/bh3/en-us/assets.json")
        );
    }
}
