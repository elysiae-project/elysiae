//! Core game installer module for Sophon chunk-based downloads.
mod adaptive_assembly;
mod adaptive_download;
mod api;
mod assembly;
mod cache;
mod download;
mod error;
mod game_filters;
mod handle;
mod hdiffpatch;
mod installer;
mod plugin_api;
mod plugin_install;
mod preinstall;
mod update;

#[cfg(test)]
mod bench_tests;
#[cfg(test)]
mod integration_tests;

/// Maximum retry attempts for failed chunk downloads.
pub const MAX_RETRIES: u32 = 4;
/// Maximum concurrent file assembly tasks.
pub const ASSEMBLY_CONCURRENCY: usize = 8;
/// Size of the channel buffer for assembly task scheduling.
pub const ASSEMBLY_CHANNEL_SIZE: usize = ASSEMBLY_CONCURRENCY * 4;
/// Filename for the installed version marker file.
pub const VERSION_FILE_NAME: &str = ".sophon_version";
/// Filename for the MD5 verification cache.
pub const VERIFICATION_CACHE_FILE: &str = ".sophon_verify_cache";

/// Buffer size for file writes during assembly (256 KiB).
#[allow(dead_code)]
pub const DOWNLOAD_STREAM_BUFFER_SIZE: usize = 256 * 1024;
/// Buffer size for file writes during assembly (256 KiB).
pub const FILE_WRITE_BUFFER_SIZE: usize = 256 * 1024;
/// Buffer size for MD5 hashing (256 KiB — optimized for sequential reads).
pub const MD5_HASH_BUFFER_SIZE: usize = 256 * 1024;

/// Minimum interval between progress updates (ms).
pub const PROGRESS_UPDATE_INTERVAL_MS: u64 = 1000;

/// Minimum concurrent downloads in adaptive mode.
pub const ADAPTIVE_MIN_CONCURRENCY: usize = 8;
/// Maximum concurrent downloads in adaptive mode.
/// Computed as sqrt(available_cpu_cores) clamped to [2, 32].
pub fn adaptive_max_concurrency() -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (cpus as f64).sqrt().clamp(2.0, 32.0) as usize
}
/// Initial concurrent downloads in adaptive mode.
pub const ADAPTIVE_INITIAL_CONCURRENCY: usize = 16;
/// Time window for throughput measurement (seconds).
pub const ADAPTIVE_WINDOW_SECS: u64 = 2;

pub const FRONT_DOOR_URL: &str = concat!(
    "https://sg-hyp-api.hoyoverse.com",
    "/hyp/hyp-connect/api/getGameBranches?launcher_id=VYTpXlbWo8"
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

/// Reads the installed version tag from the game directory, if present.
pub fn read_installed_tag(game_dir: &Path) -> Option<String> {
    fs::read_to_string(version_file_path(game_dir))
        .ok()
        .map(|s| s.trim().to_owned())
}

/// Writes the installed version tag to the game directory.
pub fn write_installed_tag(game_dir: &Path, tag: &str) -> io::Result<()> {
    fs::write(version_file_path(game_dir), tag)
}

pub use error::SophonError;
pub use handle::DownloadHandle;
pub use installer::{
    InstallCallbacks, InstallOptions, ResumeContext, StateSaver, build_installers,
    build_update_installers, install, verify_integrity,
};
pub use plugin_install::{install_channel_sdks, install_plugins};
#[allow(unused_imports)]
pub use preinstall::{
    PatchAssetInfo, PatchMethod, PreinstallState, apply_preinstall, build_preinstall_plan,
    preinstall_download,
};
pub use update::{UpdateInfo, check_update};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn read_installed_tag_present() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(VERSION_FILE_NAME), "1.2.3").unwrap();
        assert_eq!(read_installed_tag(dir.path()), Some("1.2.3".to_string()));
    }

    #[test]
    fn read_installed_tag_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_installed_tag(dir.path()), None);
    }

    #[test]
    fn write_read_installed_tag_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        write_installed_tag(dir.path(), "2.0.0").unwrap();
        assert_eq!(read_installed_tag(dir.path()), Some("2.0.0".to_string()));
    }
}
