//! Core game installer module for Sophon chunk-based downloads.

mod adaptive;
mod api;
mod assembly;
mod cache;
mod download;
mod error;
mod handle;
mod installer;
mod update;

pub const MAX_RETRIES: u32 = 4;
pub const ASSEMBLY_CONCURRENCY: usize = 4;
pub const ASSEMBLY_CHANNEL_SIZE: usize = ASSEMBLY_CONCURRENCY * 4;
pub const VERSION_FILE_NAME: &str = ".sophon_version";
pub const VERIFICATION_CACHE_FILE: &str = ".sophon_verify_cache";

pub const DOWNLOAD_STREAM_BUFFER_SIZE: usize = 256 * 1024;
pub const FILE_WRITE_BUFFER_SIZE: usize = 1024 * 1024;
pub const MD5_HASH_BUFFER_SIZE: usize = 1024 * 1024;

pub const PROGRESS_UPDATE_INTERVAL_MS: u64 = 1000;

pub const ADAPTIVE_MIN_CONCURRENCY: usize = 4;
pub const ADAPTIVE_MAX_CONCURRENCY: usize = 32;
pub const ADAPTIVE_INITIAL_CONCURRENCY: usize = 8;
pub const ADAPTIVE_WINDOW_SECS: u64 = 2;

pub const FRONT_DOOR_URL: &str = concat!(
    "https://sg-hyp-api.hoyoverse.com",
    "/hyp/hyp-connect/api/getGameBranches?&launcher_id=VYTpXlbWo8"
);
pub const SOPHON_BUILD_URL_BASE: &str = concat!(
    "https://sg-public-api.hoyoverse.com",
    "/downloader/sophon_chunk/api/getBuild"
);

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn version_file_path(game_dir: &Path) -> PathBuf {
    game_dir.join(VERSION_FILE_NAME)
}

pub fn read_installed_tag(game_dir: &Path) -> Option<String> {
    fs::read_to_string(version_file_path(game_dir))
        .ok()
        .map(|s| s.trim().to_owned())
}

pub fn write_installed_tag(game_dir: &Path, tag: &str) -> io::Result<()> {
    fs::write(version_file_path(game_dir), tag)
}

pub use handle::DownloadHandle;
pub use installer::{
    apply_preinstall, build_installers, build_preinstall_installers, build_update_installers,
    install,
};
pub use update::{UpdateInfo, check_update};
