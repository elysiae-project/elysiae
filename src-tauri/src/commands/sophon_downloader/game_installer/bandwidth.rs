//! Bandwidth manager for real-time speed tracking.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Time window for bandwidth averaging (seconds).
const BANDWIDTH_WINDOW_SECS: u64 = 3;

/// A single bandwidth sample point.
#[derive(Debug, Clone)]
struct BandwidthSample {
    timestamp: Instant,
    #[allow(dead_code)]
    bytes: u64,
}

/// Bandwidth tracker for a single stream.
struct BandwidthTracker {
    window: Mutex<VecDeque<BandwidthSample>>,
    total_bytes: AtomicU64,
}

impl BandwidthTracker {
    fn new() -> Self {
        Self {
            window: Mutex::new(VecDeque::new()),
            total_bytes: AtomicU64::new(0),
        }
    }

    /// Record `bytes` downloaded/written at the current time.
    fn record(&self, bytes: u64) {
        self.total_bytes.fetch_add(bytes, Ordering::Relaxed);
        let now = Instant::now();
        let mut window = self.window.lock().unwrap();
        window.push_back(BandwidthSample {
            timestamp: now,
            bytes,
        });

        let cutoff = now - Duration::from_secs(BANDWIDTH_WINDOW_SECS);
        while let Some(sample) = window.front() {
            if sample.timestamp < cutoff {
                window.pop_front();
            } else {
                break;
            }
        }
    }

    #[cfg(test)]
    fn current_speed(&self) -> f64 {
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

    #[cfg(test)]
    fn total_bytes(&self) -> u64 {
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
    download_tracker: BandwidthTracker,
    write_tracker: BandwidthTracker,
}

impl BandwidthManager {
    pub fn new() -> Self {
        Self {
            download_tracker: BandwidthTracker::new(),
            write_tracker: BandwidthTracker::new(),
        }
    }

    /// Record download bytes.
    pub fn record_download(&self, bytes: u64) {
        self.download_tracker.record(bytes);
    }

    /// Record written bytes.
    pub fn record_write(&self, bytes: u64) {
        self.write_tracker.record(bytes);
    }

    /// Format bytes/sec as human-readable string (e.g., "1.5 MB/s").
    #[cfg(test)]
    fn format_speed(speed_bps: f64) -> String {
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
    fn bandwidth_manager_records_download_and_write() {
        let manager = BandwidthManager::new();
        manager.record_download(1000);
        manager.record_write(500);
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
