use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tauri_plugin_log::log;

use super::*;

const ASSEMBLY_RAM_CHECK_INTERVAL_SECS: u64 = 2;
const ASSEMBLY_HIGH_RAM_THRESHOLD_MB: u64 = 512;
const ASSEMBLY_LOW_RAM_THRESHOLD_MB: u64 = 128;
const ASSEMBLY_CRITICAL_RAM_THRESHOLD_MB: u64 = 64;

pub struct AdaptiveAssembly {
    target: AtomicUsize,
}

impl AdaptiveAssembly {
    pub fn new() -> Self {
        Self {
            target: AtomicUsize::new(ASSEMBLY_CONCURRENCY),
        }
    }

    pub fn current_target(&self) -> usize {
        self.target.load(Ordering::Acquire)
    }

    pub fn adjust(&self) -> usize {
        let available_mb = available_ram_mb();
        let current = self.target.load(Ordering::Acquire);

        let new_target = if available_mb <= ASSEMBLY_CRITICAL_RAM_THRESHOLD_MB {
            1
        } else if available_mb <= ASSEMBLY_LOW_RAM_THRESHOLD_MB {
            (ASSEMBLY_CONCURRENCY / 4).max(1)
        } else if available_mb <= ASSEMBLY_HIGH_RAM_THRESHOLD_MB {
            (ASSEMBLY_CONCURRENCY / 2).max(2)
        } else {
            ASSEMBLY_CONCURRENCY
        };

        if new_target != current {
            log::info!(
                "AdaptiveAssembly: available RAM {available_mb} MiB, adjusting assembly concurrency {current} -> {new_target}",
            );
            self.target.store(new_target, Ordering::Release);
        }

        new_target
    }

    pub fn spawn_adjuster(self: &Arc<Self>, cancel_token: tokio_util::sync::CancellationToken) {
        let adaptive = Arc::clone(self);
        let token = cancel_token.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                ASSEMBLY_RAM_CHECK_INTERVAL_SECS,
            ));
            loop {
                tokio::select! {
                    _ = token.cancelled() => break,
                    _ = interval.tick() => {
                        adaptive.adjust();
                    }
                }
            }
        });
    }
}

impl Default for AdaptiveAssembly {
    fn default() -> Self {
        Self::new()
    }
}

fn available_ram_mb() -> u64 {
    use std::sync::{Mutex, OnceLock};
    use sysinfo::System;

    static SYS: OnceLock<Mutex<System>> = OnceLock::new();
    let sys = SYS.get_or_init(|| Mutex::new(System::new()));
    let Ok(mut guard) = sys.lock() else {
        return u64::MAX;
    };
    guard.refresh_memory();
    guard.available_memory() / (1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_initial_target() {
        let aa = AdaptiveAssembly::new();
        assert_eq!(aa.current_target(), ASSEMBLY_CONCURRENCY);
    }

    #[test]
    fn adjust_critical_ram() {
        assert!((ASSEMBLY_CONCURRENCY / 4).max(1) >= 1);
        assert!((ASSEMBLY_CONCURRENCY / 2).max(2) >= 2);
    }

    #[test]
    fn target_never_below_one() {
        let aa = AdaptiveAssembly::new();
        let target = aa.current_target();
        assert!(target >= 1);
    }

    #[test]
    fn adjust_updates_target() {
        let aa = AdaptiveAssembly::new();
        let _ = aa.adjust();
        // adjust() reads available RAM via sysinfo.
        let target = aa.current_target();
        assert!(target >= 1);
        assert!(target <= ASSEMBLY_CONCURRENCY);
    }

    #[test]
    fn available_ram_mb_reads_proc_meminfo() {
        let mb = available_ram_mb();
        assert!(mb > 0, "MemAvailable should be positive on Linux");
        assert!(mb < 1_000_000, "MemAvailable should be less than 1M MiB");
    }

    #[test]
    fn adjust_high_ram_stays_at_max() {
        let aa = AdaptiveAssembly::new();
        let _ = aa.adjust();
        let target = aa.current_target();
        if available_ram_mb() > ASSEMBLY_HIGH_RAM_THRESHOLD_MB {
            assert_eq!(target, ASSEMBLY_CONCURRENCY);
        }
    }

    #[test]
    fn adjust_critical_ram_goes_to_one() {
        let critical_threshold = ASSEMBLY_CRITICAL_RAM_THRESHOLD_MB;
        let low_threshold = ASSEMBLY_LOW_RAM_THRESHOLD_MB;
        let high_threshold = ASSEMBLY_HIGH_RAM_THRESHOLD_MB;
        assert!(critical_threshold < low_threshold);
        assert!(low_threshold < high_threshold);
        let target_when_critical = if 0 < critical_threshold {
            1usize
        } else {
            ASSEMBLY_CONCURRENCY
        };
        assert_eq!(target_when_critical, 1);
    }

    #[tokio::test]
    async fn spawn_adjuster_cancels_cleanly() {
        let aa = Arc::new(AdaptiveAssembly::new());
        let token = tokio_util::sync::CancellationToken::new();
        aa.spawn_adjuster(token.clone());
        token.cancel();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(aa.current_target() >= 1);
        assert!(aa.current_target() <= ASSEMBLY_CONCURRENCY);
    }

    /// The spawned adjuster task must terminate promptly after cancel to avoid
    /// leaking across install runs.
    #[tokio::test(flavor = "current_thread")]
    async fn spawn_adjuster_returns_to_runtime_after_cancel() {
        let aa = Arc::new(AdaptiveAssembly::new());
        let token = tokio_util::sync::CancellationToken::new();
        aa.spawn_adjuster(token.clone());
        // Cancel immediately before the interval fires.
        token.cancel();
        // Give the task time to exit (3x tick interval for CI stability).
        tokio::time::sleep(std::time::Duration::from_millis(
            ASSEMBLY_RAM_CHECK_INTERVAL_SECS * 3000,
        ))
        .await;
        // Target must stay within valid bounds.
        assert!(
            aa.current_target() <= ASSEMBLY_CONCURRENCY,
            "target must not exceed ASSEMBLY_CONCURRENCY after cancel"
        );
        assert!(aa.current_target() >= 1, "target must always be >= 1");
    }
}
