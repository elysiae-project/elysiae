use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;

use crate::{
    core::{
        fs::{full_path, read_dir, remove},
        game::Game,
    }, util::{cache::AssetType::{Image, Video}, web::{download_file, fetch_data}},
};

pub enum AssetType {
    Image,
    Video,
    Icon,
    Shortcut,
    Overlay,
}

#[derive(Debug, Deserialize)]
struct AedesResponse {
    backgrounds: Vec<AedesBackgroundAssets>,
    icon: String,
    icon_cn: String,
    shortcut: String,
    shortcut_cn: String,
}

#[derive(Debug, Deserialize)]
struct AedesBackgroundAssets {
    image: String,
    video: Option<String>,
    overlay: Option<String>,
}

pub async fn update_cache() -> Result<()> {
    let games = vec![
        Game::try_from("bh3")?,
        Game::try_from("hk4e")?,
        Game::try_from("hkrpg")?,
        Game::try_from("nap")?,
    ];
    let locale = "en-us"; // TODO: Use settings-based locale later

    for game in games {
        let mut downloaded: Vec<PathBuf> = vec![];

        let files_present: Vec<PathBuf> = read_dir(
            PathBuf::from(format!("cache/{}/{}", game.code(), locale)),
            None,
        )?;

        let url = format!(
            "https://aedes.elysiae.app/v3/getAssets?game={}&locale={}",
            game.code(),
            locale
        );
        let response = fetch_data::<AedesResponse>(&url).await?;
        for vs in response.asset_paths() {
            let p = PathBuf::from(format!("cache{vs}")); // Should return cache/game code/locale/filename.ext

            // Should not continue if the value is not defined or is empty, and there is no
            // need to continue if the file already exists
            if vs.is_empty() || files_present.contains(&p) {
                continue;
            }
            downloaded.push(full_path(Some(p.clone()), None)?);
            let url = format!("https://aedes.elysiae.app{vs}"); // v_str contains the forwards slash omitted in the url here

            download_file(url, p, None, None).await?;

            // TODO: File hash verification
        }

        // Assemble a list of files that are no longer on the Aedes endpoint and delete
        // them
        let to_delete: Vec<PathBuf> = files_present
            .iter()
            .filter(|x| !downloaded.contains(x))
            .cloned()
            .collect();

        for file in to_delete {
            remove(file, None, Some(true))?;
        }
    }

    Ok(())
}

impl AedesResponse {
    fn asset_paths(&self) -> impl Iterator<Item = &str> {
        self.backgrounds
            .iter()
            .flat_map(|background| {
                std::iter::once(background.image.as_str())
                    .chain(background.video.as_deref())
                    .chain(background.overlay.as_deref())
            })
            .chain([
                self.icon.as_str(),
                self.icon_cn.as_str(),
                self.shortcut.as_str(),
                self.shortcut_cn.as_str(),
            ])
    }
}

pub fn get_cached_asset_paths(game: Game, locale: &str, asset_type: AssetType) -> Result<Vec<PathBuf>> {
    let mut res: Vec<PathBuf> = vec![];
    
    todo!()
}
