use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
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
    let games = [Game::Bh3, Game::Hk4e, Game::Hkrpg, Game::Nap];
    let locale = "en-us";

    for game in games {
        let url = format!(
            "https://aedes.elysiae.app/getAssets?lang={locale}&game={}",
            game.code()
        );
        let response = fetch_data::<AedesResponse>(&url).await?;
        write_file(
            metadata_path(game, locale),
            &serde_json::to_vec_pretty(&response)?,
            None,
        )?;

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
    }

    Ok(())
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
        .map(|path| cache_path(&PathBuf::from(path)).and_then(|path| full_path(Some(path), None)))
        .collect()
}
