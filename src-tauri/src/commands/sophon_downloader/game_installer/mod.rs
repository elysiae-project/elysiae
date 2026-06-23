//! Core game installer module for Sophon chunk-based downloads.
mod adaptive_assembly;
mod adaptive_download;
mod api;
mod assembly;
mod assembly_opt;
mod bandwidth;
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
pub const MAX_RETRIES: u32 = 5;
pub const MAX_HASH_RETRIES: u32 = 5;

/// Streaming-download idle-poll interval. The HTTP body streaming loop wakes
/// at this cadence to re-check cancellation and pause state. Must be small
/// enough that a stalled connection cannot delay user-initiated cancel/pause
/// past this bound on the order of seconds.
pub const STREAM_POLL_INTERVAL_MS: u64 = 1_000;

use std::time::Duration;

pub fn retry_delay(attempt: u32) -> Duration {
    let exp = 1000u64.saturating_mul(1u64 << attempt.min(5));
    // Add jitter to prevent thundering herd when multiple chunks fail
    // simultaneously Use timestamp-based pseudo-random jitter
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let jitter = (seed.wrapping_mul(1103515245)).wrapping_add(12345) % 1000;
    Duration::from_millis(exp.min(30_000) + jitter)
}

pub async fn cancelable_sleep(
    handle: &crate::commands::sophon_downloader::game_installer::handle::DownloadHandle,
    delay: Duration,
) -> Result<(), ()> {
    tokio::select! {
        _ = tokio::time::sleep(delay) => Ok(()),
        _ = async {
            loop {
                if handle.is_cancelled() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        } => Err(()),
    }
}
/// Maximum concurrent file assembly tasks.
pub const ASSEMBLY_CONCURRENCY: usize = 8;
/// Size of the channel buffer for assembly task scheduling.
pub const ASSEMBLY_CHANNEL_SIZE: usize = ASSEMBLY_CONCURRENCY * 4;
/// Filename for the installed version marker file.
pub const VERSION_FILE_NAME: &str = ".sophon_version";
/// Filename for the MD5 verification cache.
pub const VERIFICATION_CACHE_FILE: &str = ".sophon_verify_cache";

/// Buffer size for file writes during assembly (256 KiB).
pub const FILE_WRITE_BUFFER_SIZE: usize = 256 * 1024;

/// Minimum interval between progress updates (ms).
pub const PROGRESS_UPDATE_INTERVAL_MS: u64 = 1000;

/// Minimum concurrent downloads in adaptive mode.
pub const ADAPTIVE_MIN_CONCURRENCY: usize = 8;
/// Maximum concurrent downloads in adaptive mode.
/// Computed as (cores * 4) clamped to [8, 128].
pub fn adaptive_max_concurrency() -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (cpus * 4).clamp(8, 128)
}
/// Initial concurrent downloads in adaptive mode.
pub const ADAPTIVE_INITIAL_CONCURRENCY: usize = 32;
/// Time window for throughput measurement (seconds).
pub const ADAPTIVE_WINDOW_SECS: u64 = 3;

pub const FRONT_DOOR_URL: &str = concat!(
    "\x68\x74\x74\x70\x73\x3a\x2f\x2f\x73\x67\x2d\x68\x79\x70\x2d\x61\x70\x69\x2e\x68\x6f\x79\x6f\x76\x65\x72\x73\x65\x2e\x63\x6f\x6d",
    "\x2f\x68\x79\x70\x2f\x68\x79\x70\x2d\x63\x6f\x6e\x6e\x65\x63\x74",
    "/api/getGameBranches?launcher_id=VYTpXlbWo8"
);
pub const SOPHON_BUILD_URL_BASE: &str = concat!(
    "\x68\x74\x74\x70\x73\x3a\x2f\x2f\x73\x67\x2d\x70\x75\x62\x6c\x69\x63\x2d\x61\x70\x69\x2e\x68\x6f\x79\x6f\x76\x65\x72\x73\x65\x2e\x63\x6f\x6d",
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

pub use assembly::validate_asset_name;
pub use error::SophonError;
pub use handle::DownloadHandle;
pub use installer::{
    InstallCallbacks, InstallOptions, ResumeContext, StateSaver, build_installers,
    build_update_installers, install, verify_integrity,
};
pub use plugin_install::{install_channel_sdks, install_plugins};
pub use preinstall::{apply_preinstall, build_preinstall_plan, preinstall_download};
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

    #[test]
    fn adaptive_max_concurrency_bounds() {
        let c = adaptive_max_concurrency();
        assert!(c >= 8, "concurrency {c} < 8");
        assert!(c <= 128, "concurrency {c} > 128");
    }

    #[test]
    fn adaptive_max_concurrency_formula() {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let expected = (cpus * 4).clamp(8, 128);
        assert_eq!(adaptive_max_concurrency(), expected);
    }

    #[test]
    fn retry_delay_exponential() {
        // Base delays with jitter (0-999ms added)
        assert!(retry_delay(0) >= Duration::from_millis(1000));
        assert!(retry_delay(0) < Duration::from_millis(2000));
        assert!(retry_delay(1) >= Duration::from_millis(2000));
        assert!(retry_delay(1) < Duration::from_millis(3000));
        assert!(retry_delay(2) >= Duration::from_millis(4000));
        assert!(retry_delay(2) < Duration::from_millis(5000));
        assert!(retry_delay(3) >= Duration::from_millis(8000));
        assert!(retry_delay(3) < Duration::from_millis(9000));
        assert!(retry_delay(4) >= Duration::from_millis(16000));
        assert!(retry_delay(4) < Duration::from_millis(17000));
    }

    #[test]
    fn retry_delay_capped_at_30s() {
        // Capped at 30000 + jitter (up to ~30999ms)
        assert!(retry_delay(5) >= Duration::from_millis(30000));
        assert!(retry_delay(5) < Duration::from_millis(31000));
        assert!(retry_delay(10) >= Duration::from_millis(30000));
        assert!(retry_delay(10) < Duration::from_millis(31000));
        assert!(retry_delay(100) >= Duration::from_millis(30000));
        assert!(retry_delay(100) < Duration::from_millis(31000));
    }

    #[tokio::test]
    async fn cancelable_sleep_completes_normally() {
        let handle = DownloadHandle::new();
        let result = cancelable_sleep(&handle, Duration::from_millis(10)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cancelable_sleep_returns_err_on_cancel() {
        let handle = DownloadHandle::new();
        handle.cancel();
        let result = cancelable_sleep(&handle, Duration::from_secs(30)).await;
        assert!(result.is_err());
    }
}
