mod adaptive;
mod api;
mod assembly;
mod cache;
mod constants;
mod download;
mod handle;
mod installer;
mod manifest;
mod update;
mod version;

pub use handle::DownloadHandle;
pub use installer::{
    apply_preinstall, build_installers, build_preinstall_installers, build_update_installers,
    install,
};
pub use update::{UpdateInfo, check_update};
pub use version::read_installed_tag;
