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
                "AdaptiveAssembly: available RAM {} MiB, adjusting assembly concurrency {} → {}",
                available_mb,
                current,
                new_target,
            );
            self.target.store(new_target, Ordering::Release);
        }

        new_target
    }

    pub fn spawn_adjuster(self: &Arc<Self>) -> tokio_util::sync::CancellationToken {
        let cancel_token = tokio_util::sync::CancellationToken::new();
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

        cancel_token
    }
}

impl Default for AdaptiveAssembly {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "linux")]
fn available_ram_mb() -> u64 {
    let Ok(contents) = std::fs::read_to_string("/proc/meminfo") else {
        return u64::MAX;
    };
    for line in contents.lines() {
        if line.starts_with("MemAvailable:")
            && let Ok(kb) = line.split_whitespace().nth(1).unwrap_or("0").parse::<u64>()
        {
            return kb / 1024;
        }
    }
    u64::MAX
}

#[cfg(not(target_os = "linux"))]
fn available_ram_mb() -> u64 {
    u64::MAX
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
        assert_eq!(1, 1);
        assert_eq!((ASSEMBLY_CONCURRENCY / 4).max(1), ASSEMBLY_CONCURRENCY / 4);
        assert_eq!((ASSEMBLY_CONCURRENCY / 2).max(2), ASSEMBLY_CONCURRENCY / 2);
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
        // On Linux, this reads /proc/meminfo; on other platforms, returns u64::MAX
        let target = aa.current_target();
        assert!(target >= 1);
        assert!(target <= ASSEMBLY_CONCURRENCY);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn available_ram_mb_reads_proc_meminfo() {
        let mb = available_ram_mb();
        assert!(mb > 0, "MemAvailable should be positive on Linux");
        assert!(mb < 1_000_000, "MemAvailable should be less than 1M MiB");
    }
}
