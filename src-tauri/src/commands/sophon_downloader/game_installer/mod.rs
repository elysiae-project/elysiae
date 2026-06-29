//! Sophon chunk-based game installer.
mod adaptive_assembly;
mod api;
mod assembly;
mod assembly_opt;
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

mod profiling;

#[cfg(test)]
mod bench_tests;
#[cfg(test)]
mod integration_tests;

/// Max retries for failed chunk downloads.
pub const MAX_RETRIES: u32 = 5;
pub const MAX_HASH_RETRIES: u32 = 5;

/// How often the download loop polls for cancellation/pause (ms).
pub const STREAM_POLL_INTERVAL_MS: u64 = 250;

use std::time::Duration;

pub fn retry_delay(attempt: u32) -> Duration {
    let exp = 1000u64.saturating_mul(1u64 << attempt.min(5));
    // Jitter prevents thundering-herd when multiple chunks fail at once.
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
/// Max concurrent assembly tasks.
pub const ASSEMBLY_CONCURRENCY: usize = 4;
/// Assembly task channel buffer size.
pub const ASSEMBLY_CHANNEL_SIZE: usize = 4096;
/// Installed version marker filename.
pub const VERSION_FILE_NAME: &str = ".sophon_version";
/// MD5 verification cache filename.
pub const VERIFICATION_CACHE_FILE: &str = ".sophon_verify_cache";

/// File write buffer during assembly.
pub const FILE_WRITE_BUFFER_SIZE: usize = 256 * 1024;
/// File write buffer during chunk downloads.
pub const CHUNK_WRITE_BUFFER_SIZE: usize = 256 * 1024;

/// Minimum progress update interval (ms).
pub const PROGRESS_UPDATE_INTERVAL_MS: u64 = 1000;

/// Download speed smoothing window (seconds).
pub const SPEED_SMOOTH_WINDOW_SECS: f64 = 5.0;

/// Update an EWMA-smoothed value stored as a scaled u64.
pub fn ewma_update(atomic: &std::sync::atomic::AtomicU64, raw_value: f64, alpha: f64) -> f64 {
    const SCALE: f64 = 1000.0;
    use std::sync::atomic::Ordering;
    let prev_raw = atomic.load(Ordering::Relaxed);
    let prev = prev_raw as f64 / SCALE;
    let new_val = if prev == 0.0 {
        raw_value
    } else {
        alpha * raw_value + (1.0 - alpha) * prev
    };
    atomic.store((new_val * SCALE) as u64, Ordering::Release);
    new_val
}

/// ETA speed sample window size.
pub const ETA_WINDOW_SAMPLES: usize = 30;
/// Minimum samples before ETA is displayed.
pub const ETA_MIN_SAMPLES: usize = 5;

/// Compute ETA speed using median filtering over recent speed samples.
/// Returns 0.0 if fewer than `ETA_MIN_SAMPLES` are available.
pub fn compute_eta_speed(
    history: &std::sync::Mutex<std::collections::VecDeque<f64>>,
    instant_speed: f64,
) -> f64 {
    let mut samples = history.lock().unwrap_or_else(|err| err.into_inner());
    samples.push_back(instant_speed);
    if samples.len() > ETA_WINDOW_SAMPLES {
        samples.pop_front();
    }
    if samples.len() < ETA_MIN_SAMPLES {
        return 0.0;
    }
    let mut sorted: Vec<f64> = samples.iter().copied().collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

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

/// Read the installed version tag, if present.
pub fn read_installed_tag(game_dir: &Path) -> Option<String> {
    fs::read_to_string(version_file_path(game_dir))
        .ok()
        .map(|s| s.trim().to_owned())
}

/// Write the installed version tag.
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

    #[test]
    fn ewma_update_first_sample_is_raw() {
        let atomic = std::sync::atomic::AtomicU64::new(0);
        let val = ewma_update(&atomic, 1000.0, 0.2);
        assert!((val - 1000.0).abs() < 0.01);
    }

    #[test]
    fn ewma_update_smoothing_converges() {
        let atomic = std::sync::atomic::AtomicU64::new(0);
        ewma_update(&atomic, 1000.0, 0.2);
        let val = ewma_update(&atomic, 500.0, 0.2);
        assert!((val - 900.0).abs() < 0.01);
        let val = ewma_update(&atomic, 500.0, 0.2);
        assert!(val < 900.0 && val > 500.0);
    }

    #[test]
    fn ewma_update_speed_window_alpha() {
        let alpha = 1.0 / (SPEED_SMOOTH_WINDOW_SECS * 1000.0 / PROGRESS_UPDATE_INTERVAL_MS as f64);
        assert!((alpha - 0.2).abs() < 0.001);
    }

    #[test]
    fn compute_eta_speed_returns_zero_with_few_samples() {
        let history = std::sync::Mutex::new(std::collections::VecDeque::new());
        assert_eq!(compute_eta_speed(&history, 1000.0), 0.0);
        assert_eq!(compute_eta_speed(&history, 2000.0), 0.0);
        assert_eq!(compute_eta_speed(&history, 3000.0), 0.0);
        assert_eq!(compute_eta_speed(&history, 4000.0), 0.0);
    }

    #[test]
    fn compute_eta_speed_median_with_enough_samples() {
        let history = std::sync::Mutex::new(std::collections::VecDeque::new());
        for _ in 0..4 {
            compute_eta_speed(&history, 1000.0);
        }
        let result = compute_eta_speed(&history, 1000.0);
        assert!((result - 1000.0).abs() < 0.01);
    }

    #[test]
    fn compute_eta_speed_median_rejects_outlier() {
        let history = std::sync::Mutex::new(std::collections::VecDeque::new());
        for _ in 0..4 {
            compute_eta_speed(&history, 100.0);
        }
        let result = compute_eta_speed(&history, 10000.0);
        assert!((result - 100.0).abs() < 0.01);
    }

    #[test]
    fn compute_eta_speed_window_bounds() {
        let history = std::sync::Mutex::new(std::collections::VecDeque::new());
        for i in 0..ETA_WINDOW_SAMPLES + 5 {
            compute_eta_speed(&history, i as f64 * 100.0);
        }
        let guard = history.lock().unwrap();
        assert!(guard.len() <= ETA_WINDOW_SAMPLES);
    }
}
