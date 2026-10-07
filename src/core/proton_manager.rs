use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use log::info;
use serde::{Deserialize, Serialize};

use crate::{
    core::fs::{
        MultiPathOptions, exists, extract_file, full_path, mkdir, read_file, remove,
        verify_sha256sum, write_file,
    },
    util::{
        shell::exec_shell,
        web::{download_file, fetch_data},
    },
};

const COMPONENTS_URL_BASE: &str = "https://aedes.elysiae.app/getComponentInfo";
#[cfg(target_arch = "x86_64")]
const ARCH: &str = "amd64";
#[cfg(target_arch = "aarch64")]
const ARCH: &str = "aarch64";
const MAX_RETRIES: i32 = 5;

pub struct GameModule {
    component_name: String,
    extract_to: PathBuf,
    save_to: PathBuf,
    tracker_file_name: PathBuf,
    post_install: Option<Box<dyn Fn() -> Result<()>>>,
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
    prerelease: bool,
    download: ComponentDownload,
}

#[derive(Debug, Serialize, Deserialize)]
struct ComponentDownload {
    url: String,
    checksum: String,
}

impl GameModule {
    /// Creates a new GameModule instance
    pub fn new(
        component_name: String,
        extract_to: PathBuf,
        save_to: PathBuf,
        tracker_file_name: PathBuf,
        post_install: Option<Box<dyn Fn() -> Result<()>>>,
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

        let mut url = reqwest::Url::parse(COMPONENTS_URL_BASE)?;
        url.query_pairs_mut()
            .append_pair("arch", ARCH)
            .append_pair("component", &self.component_name)
            .append_pair("latestOnly", "true");
        let latest_release = fetch_data::<ComponentRelease>(url.as_str()).await?;
        if latest_release.prerelease {
            anyhow::bail!("Refusing prerelease component {}", self.component_name);
        }
        if self.should_update(&latest_release)? {
            let checksum = latest_release.download.checksum.trim().to_ascii_lowercase();
            anyhow::ensure!(
                checksum.len() == 64 && checksum.chars().all(|c| c.is_ascii_hexdigit()),
                "Invalid component checksum"
            );
            // Get the latest release url and checksum, then download the file
            let mut remaining_attempts = MAX_RETRIES;
            let latest_release_url = latest_release.download.url;

            while remaining_attempts > 0 {
                let attempt = async {
                    download_file(
                        latest_release_url.clone(),
                        self.save_to.clone(),
                        None,
                        Some(Box::new(|progress| {
                            info!(
                                "{}/{} ({}%)",
                                progress.downloaded,
                                progress.total,
                                progress.downloaded.saturating_mul(100) / progress.total.max(1)
                            );
                        })),
                    )
                    .await?;
                    anyhow::ensure!(
                        verify_sha256sum(self.save_to.clone(), None, checksum.clone()).await?,
                        "Downloaded component checksum did not match"
                    );
                    Ok::<_, anyhow::Error>(())
                }
                .await;

                if attempt.is_ok() {
                    break;
                }
                remaining_attempts -= 1;
                let _ = remove(self.save_to.clone(), None, None);
                if remaining_attempts > 0 {
                    tokio::time::sleep(Duration::from_millis(
                        250 * (MAX_RETRIES - remaining_attempts) as u64,
                    ))
                    .await;
                }
            }

            if remaining_attempts == 0 {
                return Err(anyhow::anyhow!(
                    "Failed to download and verify {} after {} attempts",
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
            )
            .await?;

            remove(self.save_to.clone(), None, None)?;

            // Perform post-install actions, if any
            if let Some(post_install) = &self.post_install {
                post_install()?;
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
        String::from("phlogiston"),
        PathBuf::from("phlogiston"),
        PathBuf::from("phlogiston.tar.gz"),
        PathBuf::from("proton.json"),
        Some(Box::new(|| mkdir(PathBuf::from("proton-data"), None))),
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
    let proton = full_path(Some(PathBuf::from("proton")), None)?;
    if proton.is_file() {
        return Ok(!proton.is_symlink()
            && std::fs::metadata(&proton)
                .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
                .unwrap_or(false));
    }
    if proton.is_dir() && !proton.is_symlink() {
        let executable = proton.join("proton");
        return Ok(executable.is_file()
            && !executable.is_symlink()
            && std::fs::metadata(executable)
                .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
                .unwrap_or(false));
    }
    Ok(false)
}

pub fn exec_proton(app_path: PathBuf) -> Result<std::process::Child> {
    let proton_path = full_path(Some(PathBuf::from("proton")), None)?;
    let proton_path_str = proton_path
        .to_str()
        .context("Proton path is not valid UTF-8")?;

    let fp = full_path(Some(app_path), None)?;
    let str_path = fp.to_str().context("Game path is not valid UTF-8")?;
    exec_shell(proton_path_str, &[str_path.to_owned()])
}
