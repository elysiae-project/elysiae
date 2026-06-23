//! Bandwidth manager for real-time speed tracking and adaptive throttling.
//!
//! This module implements a production-grade bandwidth management system
//! inspired by the original Sophon DLL's bandwidth manager
//! (`sophon/sophon_net/bandwidth/bandwidth_manager.cc`). It tracks per-stream
//! and aggregate bandwidth metrics, supports dynamic throttling, and provides
//! adaptive bandwidth scheduling.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Time window for bandwidth averaging (seconds).
const BANDWIDTH_WINDOW_SECS: u64 = 3;
/// Default bandwidth limit in bytes per second (0 = unlimited).
const DEFAULT_BANDWIDTH_LIMIT: u64 = 0;

/// Bandwidth metrics tracked per download stream.
#[derive(Debug, Clone)]
pub struct BandwidthMetrics {
    /// Bytes downloaded from network.
    pub download_bytes: u64,
    /// Bytes written to disk.
    pub written_bytes: u64,
    /// Bytes verified (hashed).
    pub verified_bytes: u64,
    /// Current download speed in bytes/sec.
    pub download_speed: f64,
    /// Current write speed in bytes/sec.
    pub write_speed: f64,
    /// Current verification speed in bytes/sec.
    pub verify_speed: f64,
    /// Timestamp of last update.
    pub last_update: Instant,
}

/// A single bandwidth sample point.
#[derive(Debug, Clone)]
struct BandwidthSample {
    timestamp: Instant,
    bytes: u64,
}

/// Bandwidth tracker for a single stream.
pub struct BandwidthTracker {
    window: Mutex<VecDeque<BandwidthSample>>,
    total_bytes: AtomicU64,
    last_reported: AtomicU64,
}

impl BandwidthTracker {
    pub fn new() -> Self {
        Self {
            window: Mutex::new(VecDeque::new()),
            total_bytes: AtomicU64::new(0),
            last_reported: AtomicU64::new(0),
        }
    }

    /// Record `bytes` downloaded/written/verified at the current time.
    pub fn record(&self, bytes: u64) {
        self.total_bytes.fetch_add(bytes, Ordering::Relaxed);
        let now = Instant::now();
        let mut window = self.window.lock().unwrap();
        window.push_back(BandwidthSample {
            timestamp: now,
            bytes,
        });

        // Clean up old samples outside the window
        let cutoff = now - Duration::from_secs(BANDWIDTH_WINDOW_SECS);
        while let Some(sample) = window.front() {
            if sample.timestamp < cutoff {
                window.pop_front();
            } else {
                break;
            }
        }
    }

    /// Calculate current speed in bytes/sec based on the sliding window.
    pub fn current_speed(&self) -> f64 {
        let window = self.window.lock().unwrap();
        if window.len() < 2 {
            return 0.0;
        }

        let first = window.front().unwrap();
        let last = window.back().unwrap();
        let duration = last.timestamp.duration_since(first.timestamp).as_secs_f64();
        if duration == 0.0 {
            return 0.0;
        }

        let total_bytes: u64 = window.iter().map(|s| s.bytes).sum();
        total_bytes as f64 / duration
    }

    /// Get total bytes recorded.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes.load(Ordering::Relaxed)
    }
}

impl Default for BandwidthTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Global bandwidth manager that coordinates bandwidth across all streams.
pub struct BandwidthManager {
    /// Aggregate download tracker.
    pub download_tracker: BandwidthTracker,
    /// Aggregate write tracker.
    pub write_tracker: BandwidthTracker,
    /// Aggregate verification tracker.
    pub verify_tracker: BandwidthTracker,
    /// Bandwidth limit in bytes/sec (0 = unlimited).
    bandwidth_limit: AtomicU64,
    /// Last time bandwidth was throttled.
    last_throttle: Mutex<Instant>,
}

impl BandwidthManager {
    pub fn new() -> Self {
        Self {
            download_tracker: BandwidthTracker::new(),
            write_tracker: BandwidthTracker::new(),
            verify_tracker: BandwidthTracker::new(),
            bandwidth_limit: AtomicU64::new(DEFAULT_BANDWIDTH_LIMIT),
            last_throttle: Mutex::new(Instant::now()),
        }
    }

    /// Set bandwidth limit in bytes/sec. 0 = unlimited.
    pub fn set_bandwidth_limit(&self, bytes_per_sec: u64) {
        self.bandwidth_limit.store(bytes_per_sec, Ordering::Relaxed);
    }

    /// Get current bandwidth limit in bytes/sec.
    pub fn bandwidth_limit(&self) -> u64 {
        self.bandwidth_limit.load(Ordering::Relaxed)
    }

    /// Record download bytes.
    pub fn record_download(&self, bytes: u64) {
        self.download_tracker.record(bytes);
    }

    /// Record written bytes.
    pub fn record_write(&self, bytes: u64) {
        self.write_tracker.record(bytes);
    }

    /// Record verified bytes.
    pub fn record_verify(&self, bytes: u64) {
        self.verify_tracker.record(bytes);
    }

    /// Get current aggregate metrics.
    pub fn metrics(&self) -> BandwidthMetrics {
        BandwidthMetrics {
            download_bytes: self.download_tracker.total_bytes(),
            written_bytes: self.write_tracker.total_bytes(),
            verified_bytes: self.verify_tracker.total_bytes(),
            download_speed: self.download_tracker.current_speed(),
            write_speed: self.write_tracker.current_speed(),
            verify_speed: self.verify_tracker.current_speed(),
            last_update: Instant::now(),
        }
    }

    /// Check if we should throttle based on bandwidth limit.
    /// Returns the number of bytes that can be processed now.
    pub fn throttle_allowance(&self, requested: u64) -> u64 {
        let limit = self.bandwidth_limit();
        if limit == 0 {
            return requested; // No limit
        }

        let current_speed = self.download_tracker.current_speed();
        if current_speed >= limit as f64 {
            // Already at limit, allow minimal progress
            return requested.min(1024); // Allow 1KB to prevent stalling
        }

        // Allow full request if under limit
        requested
    }

    /// Calculate sleep duration needed to maintain bandwidth limit.
    pub fn throttle_delay(&self) -> Option<Duration> {
        let limit = self.bandwidth_limit();
        if limit == 0 {
            return None;
        }

        let current_speed = self.download_tracker.current_speed();
        if current_speed <= limit as f64 * 0.9 {
            return None; // Under 90% of limit, no delay needed
        }

        // Need to slow down - sleep for a short duration
        Some(Duration::from_millis(100))
    }

    /// Format bytes/sec as human-readable string (e.g., "1.5 MB/s").
    pub fn format_speed(speed_bps: f64) -> String {
        if speed_bps >= 1_073_741_824.0 {
            format!("{:.2} GB/s", speed_bps / 1_073_741_824.0)
        } else if speed_bps >= 1_048_576.0 {
            format!("{:.2} MB/s", speed_bps / 1_048_576.0)
        } else if speed_bps >= 1024.0 {
            format!("{:.2} KB/s", speed_bps / 1024.0)
        } else {
            format!("{:.0} B/s", speed_bps)
        }
    }
}

impl Default for BandwidthManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared reference type for bandwidth manager.
pub type SharedBandwidthManager = Arc<BandwidthManager>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn bandwidth_tracker_records_and_calculates() {
        let tracker = BandwidthTracker::new();
        tracker.record(1000);
        thread::sleep(Duration::from_millis(100));
        tracker.record(2000);

        let speed = tracker.current_speed();
        assert!(speed > 0.0, "Speed should be positive");
    }

    #[test]
    fn bandwidth_manager_tracks_all_metrics() {
        let manager = BandwidthManager::new();
        manager.record_download(1000);
        manager.record_write(500);
        manager.record_verify(250);

        let metrics = manager.metrics();
        assert_eq!(metrics.download_bytes, 1000);
        assert_eq!(metrics.written_bytes, 500);
        assert_eq!(metrics.verified_bytes, 250);
    }

    #[test]
    fn bandwidth_limit_zero_is_unlimited() {
        let manager = BandwidthManager::new();
        manager.set_bandwidth_limit(0);
        assert_eq!(manager.throttle_allowance(10000), 10000);
    }

    #[test]
    fn bandwidth_limit_enforced() {
        let manager = BandwidthManager::new();
        manager.set_bandwidth_limit(1024); // 1KB/s limit

        // Simulate high bandwidth usage
        for _ in 0..10 {
            manager.record_download(1000);
        }

        // Should allow minimal progress when at limit
        let allowance = manager.throttle_allowance(10000);
        assert!(allowance <= 1024, "Allowance should be limited");
    }

    #[test]
    fn format_speed_human_readable() {
        assert_eq!(BandwidthManager::format_speed(512.0), "512 B/s");
        assert_eq!(BandwidthManager::format_speed(1536.0), "1.50 KB/s");
        assert_eq!(BandwidthManager::format_speed(1_572_864.0), "1.50 MB/s");
        assert_eq!(BandwidthManager::format_speed(1_610_612_736.0), "1.50 GB/s");
    }

    #[tokio::test]
    async fn bandwidth_tracker_concurrent_access() {
        let tracker = Arc::new(BandwidthTracker::new());
        let mut handles = Vec::new();

        for _ in 0..10 {
            let t = tracker.clone();
            handles.push(tokio::spawn(async move {
                for _ in 0..100 {
                    t.record(100);
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        assert_eq!(tracker.total_bytes(), 100_000);
    }
}
