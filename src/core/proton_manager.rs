use std::{fmt::format, path::PathBuf};

use anyhow::Result;
use log::info;
use serde::{Deserialize, Serialize};

use crate::{
    core::fs::{
        BaseDirectory::{self},
        MultiPathOptions, exists, extract_file, full_path, mkdir, read_file, remove,
        verify_sha256sum, write_file,
    },
    util::{
        shell::exec_shell,
        web::{download_file, fetch_data},
    },
};

const COMPONENTS_URL_BASE: &str = "https://aedes.elysiae.app/components/";
const ARCH: &str = std::env::consts::ARCH;
const MAX_RETRIES: i32 = 5;

pub struct GameModule {
    component_name: String,
    extract_to: PathBuf,
    save_to: PathBuf,
    tracker_file_name: PathBuf,
    post_install: Option<Box<dyn Fn()>>,
}

// Quick and dirty representation of the file structure that tracks installed
// elysiae components
#[derive(Debug, Serialize, Deserialize)]
struct InstalledComponentsData {
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ComponentRelease {
    tag: String,
    download_url: String,
    hash: String,
}

impl GameModule {
    /// Creates a new GameModule instance
    pub fn new(
        component_name: String,
        extract_to: PathBuf,
        save_to: PathBuf,
        tracker_file_name: PathBuf,
        post_install: Option<Box<dyn Fn()>>,
    ) -> Self {
        GameModule {
            component_name,
            extract_to,
            save_to,
            tracker_file_name,
            post_install,
        }
    }

    /// Checks if an update to a component should proceed by:
    ///
    /// 1: Checking if the component version tracker file exists (if it does
    /// not, the update should proceed)
    ///
    /// 2: Compare the component version in the tracker file to the one fetched
    /// from aedes.elysiae.app. If the versions do not match, an update is
    /// available, as Aedes always stores the latest version at the top of its
    /// response
    fn should_update(&mut self, release_data: &ComponentRelease) -> Result<bool> {
        // None used as the fs functions fall back to the app data dir, which is where
        // this file is meant to be saved to
        let p = PathBuf::from("components").join(&self.tracker_file_name);
        let e = exists(p.clone(), None)?;
        let latest = &release_data.tag;
        if e {
            let data = read_file(p, None)?;
            let deserialized_data = serde_json::from_slice::<InstalledComponentsData>(&data)?;

            Ok(deserialized_data.version.ne(latest))
        } else {
            Ok(true)
        }
    }

    /// Attempts to update this component. If no new release is detected, no
    /// update will be performed.
    ///
    /// After a component file is downloaded, it will be verified against its
    /// known sha256sum. If the hashes do not match, this function will delete
    /// the file and attempt another download. If the file hash is still invalid
    /// after 5 retries, the function will fail
    async fn update_module(&mut self) -> Result<()> {
        info!("Updating {}", &self.component_name);

        // TODO: Update Aedes to provide arch-based component
        // manifest files for this code to function properly

        let url = format!(
            "{}{}-{}.json",
            COMPONENTS_URL_BASE, &self.component_name, ARCH
        );
        let res = fetch_data::<Vec<ComponentRelease>>(&url).await?;
        let latest_release = res
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No Release Data Found"))?;

        if self.should_update(&latest_release)? {
            // Get the latest release url and checksum, then download the file
            let mut remaining_attempts = MAX_RETRIES;
            let latest_release_url = latest_release.download_url;
            let checksum = latest_release.hash;

            while remaining_attempts > 0 {
                download_file(
                    latest_release_url.clone(),
                    self.save_to.clone(),
                    None,
                    Some(Box::new(|progress| {
                        // TODO: make some sort of proper event handler in the UI once it gets
                        // created
                        info!(
                            "{}/{} ({}%)",
                            progress.downloaded,
                            progress.total,
                            (progress.downloaded / progress.total) * 100
                        );
                    })),
                )
                .await?;

                if verify_sha256sum(self.save_to.clone(), None, checksum.clone())? {
                    break;
                } else {
                    remaining_attempts -= 1;

                    // Remove the corrupted file
                    remove(self.save_to.clone(), None, None)?;
                }
            }

            if remaining_attempts == 0 {
                return Err(anyhow::anyhow!(
                    "Failed to verify {} after {} attempts",
                    self.component_name,
                    MAX_RETRIES
                ));
            }

            // Extract and remove the downloaded file
            extract_file(
                MultiPathOptions {
                    init_path: self.save_to.clone(),
                    init_path_base_dir: None,
                    dest_path: self.extract_to.clone(),
                    dest_path_base_dir: None,
                    overwrite: Some(true), // Replace existing files with updated ones
                },
                Some(true),
            )?;

            remove(self.save_to.clone(), None, None)?;

            // Perform post-install actions, if any
            if let Some(post_install) = &self.post_install {
                post_install();
            }

            // Update the component tracker
            self.update_component_info(latest_release.tag.clone())?;
        } else {
            info!(
                "{} is already up-to-date; skipping update.",
                self.component_name
            );
        }

        Ok(())
    }

    /// Updates this component's version tracker file with the newly installed
    /// version of the component. Files are saved as non-minified json
    fn update_component_info(&mut self, new_version: String) -> Result<()> {
        let data = InstalledComponentsData {
            version: new_version,
        };
        let str = serde_json::to_string_pretty(&data)?;
        let p = PathBuf::from("components").join(&self.tracker_file_name);
        write_file(p, str.as_bytes(), None)?;
        Ok(())
    }
}

pub async fn update_all_modules() -> Result<()> {
    // only proton exists as a game module for now, but future modules might exist
    // in the future. This code futureproofs this function for a scenario in which
    // this does happen
    let proton = GameModule::new(
        String::from("Proton"),
        PathBuf::from("proton"),
        PathBuf::from("proton.tar.gz"),
        PathBuf::from("proton.json"),
        Some(Box::new(|| {
            let _ = mkdir(PathBuf::from("proton-data"), None);
        })),
    );

    // This looks silly with only one module
    let modules = vec![proton];
    for mut module in modules {
        module.update_module().await?;
    }
    Ok(())
}

/// Checks if all components have been installed
pub fn components_installed() -> Result<bool> {
    // For now, only proton is installed, so this is an easy check to see if proton itself has been extracted to the proper directory. In the future however, if new components do get added, a more robust way to detect component installs could be useful (perhaps via an exclusive components folder and making sure it contains all components or something similar)
    Ok(exists(PathBuf::from("proton"), None)?)
}

pub fn exec_proton(app_path: PathBuf) -> Result<()> {
    let proton_path = full_path(Some(PathBuf::from("proton")), None)?;
    let proton_path_str = proton_path.to_str().unwrap();

    let fp = full_path(Some(app_path), None)?;
    let str_path = fp.to_str().unwrap();
    exec_shell(proton_path_str, &[str_path.to_owned()])?;
    Ok(())
}
