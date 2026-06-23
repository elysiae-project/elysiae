//! Download configuration for adaptive buffer sizing and I/O optimization.
//!
//! Based on the original Sophon DLL's configurable `buffer_size` and
//! `filter_source_stream_buffer_size` parameters.

use std::time::Duration;

/// Default buffer size for file writes (512 KiB).
pub const DEFAULT_FILE_WRITE_BUFFER_SIZE: usize = 512 * 1024;
/// Minimum buffer size for file writes (64 KiB).
pub const MIN_FILE_WRITE_BUFFER_SIZE: usize = 64 * 1024;
/// Maximum buffer size for file writes (2 MiB).
pub const MAX_FILE_WRITE_BUFFER_SIZE: usize = 2 * 1024 * 1024;

/// Default stream poll interval for cancellation checks (500 ms).
pub const DEFAULT_STREAM_POLL_INTERVAL_MS: u64 = 500;
/// Minimum stream poll interval (100 ms).
pub const MIN_STREAM_POLL_INTERVAL_MS: u64 = 100;
/// Maximum stream poll interval (2000 ms).
pub const MAX_STREAM_POLL_INTERVAL_MS: u64 = 2000;

/// Download configuration with adaptive buffer sizing.
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// Buffer size for file writes.
    pub file_write_buffer_size: usize,
    /// Stream poll interval for cancellation checks.
    pub stream_poll_interval_ms: u64,
    /// Whether to use adaptive buffer sizing based on bandwidth.
    pub adaptive_buffer_sizing: bool,
    /// Target buffer fill time in milliseconds.
    /// The buffer size will be adjusted so that it takes approximately
    /// this long to fill at the current bandwidth.
    pub target_buffer_fill_ms: u64,
}

impl DownloadConfig {
    /// Calculate adaptive buffer size based on current bandwidth.
    /// Returns a buffer size that takes approximately `target_buffer_fill_ms`
    /// to fill at the given bandwidth.
    pub fn adaptive_buffer_size(&self, bandwidth_bps: f64) -> usize {
        if !self.adaptive_buffer_sizing || bandwidth_bps <= 0.0 {
            return self.file_write_buffer_size;
        }

        let target_bytes = bandwidth_bps * (self.target_buffer_fill_ms as f64 / 1000.0);
        let size = target_bytes as usize;

        size.clamp(MIN_FILE_WRITE_BUFFER_SIZE, MAX_FILE_WRITE_BUFFER_SIZE)
    }

    /// Calculate adaptive poll interval based on current bandwidth.
    /// Higher bandwidth = shorter interval for more responsive cancellation.
    pub fn adaptive_poll_interval_ms(&self, bandwidth_bps: f64) -> u64 {
        if bandwidth_bps <= 0.0 {
            return self.stream_poll_interval_ms;
        }

        // Faster connections need more frequent checks
        // At 1 MB/s: 500ms
        // At 10 MB/s: 250ms
        // At 100 MB/s: 100ms
        let mbps = bandwidth_bps / 1_048_576.0;
        let interval = (500.0 / (1.0 + mbps / 10.0).sqrt()) as u64;

        interval.clamp(MIN_STREAM_POLL_INTERVAL_MS, MAX_STREAM_POLL_INTERVAL_MS)
    }
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            file_write_buffer_size: DEFAULT_FILE_WRITE_BUFFER_SIZE,
            stream_poll_interval_ms: DEFAULT_STREAM_POLL_INTERVAL_MS,
            adaptive_buffer_sizing: true,
            target_buffer_fill_ms: 100, // 100ms target fill time
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = DownloadConfig::default();
        assert_eq!(
            config.file_write_buffer_size,
            DEFAULT_FILE_WRITE_BUFFER_SIZE
        );
        assert_eq!(
            config.stream_poll_interval_ms,
            DEFAULT_STREAM_POLL_INTERVAL_MS
        );
        assert!(config.adaptive_buffer_sizing);
    }

    #[test]
    fn adaptive_buffer_size_low_bandwidth() {
        let config = DownloadConfig::default();
        // 1 MB/s bandwidth
        let size = config.adaptive_buffer_size(1_048_576.0);
        assert!(size >= MIN_FILE_WRITE_BUFFER_SIZE);
        assert!(size <= MAX_FILE_WRITE_BUFFER_SIZE);
    }

    #[test]
    fn adaptive_buffer_size_high_bandwidth() {
        let config = DownloadConfig::default();
        // 100 MB/s bandwidth
        let size = config.adaptive_buffer_size(100.0 * 1_048_576.0);
        assert!(size >= MIN_FILE_WRITE_BUFFER_SIZE);
        assert!(size <= MAX_FILE_WRITE_BUFFER_SIZE);
    }

    #[test]
    fn adaptive_buffer_size_disabled() {
        let config = DownloadConfig {
            adaptive_buffer_sizing: false,
            ..Default::default()
        };
        let size = config.adaptive_buffer_size(100.0 * 1_048_576.0);
        assert_eq!(size, DEFAULT_FILE_WRITE_BUFFER_SIZE);
    }

    #[test]
    fn adaptive_poll_interval_responsive() {
        let config = DownloadConfig::default();
        // Low bandwidth = longer interval
        let low = config.adaptive_poll_interval_ms(1_048_576.0);
        // High bandwidth = shorter interval
        let high = config.adaptive_poll_interval_ms(100.0 * 1_048_576.0);
        assert!(low > high);
        assert!(high >= MIN_STREAM_POLL_INTERVAL_MS);
        assert!(low <= MAX_STREAM_POLL_INTERVAL_MS);
    }

    #[test]
    fn adaptive_poll_interval_bounds() {
        let config = DownloadConfig::default();
        let interval = config.adaptive_poll_interval_ms(0.0);
        assert_eq!(interval, DEFAULT_STREAM_POLL_INTERVAL_MS);
    }
}
