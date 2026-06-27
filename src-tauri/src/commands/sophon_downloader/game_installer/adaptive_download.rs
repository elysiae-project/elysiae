use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use tauri_plugin_log::log;
use tokio::sync::{Notify, Semaphore, SemaphorePermit};

use super::*;

const EWMA_ALPHA: f64 = 0.25;
const THROUGHPUT_SCALE: f64 = 1000.0;
const BEST_DECAY: f64 = 0.99;

pub struct AdaptivePermit<'a> {
    _permit: SemaphorePermit<'a>,
    adaptive: &'a AdaptiveSemaphore,
}

impl Drop for AdaptivePermit<'_> {
    fn drop(&mut self) {
        self.adaptive.dec_active();
    }
}

pub struct AdaptiveSemaphore {
    semaphore: Semaphore,
    notify: Notify,
    target: AtomicUsize,
    active: AtomicUsize,
    ewma_throughput_mbps: AtomicU64,
    best_throughput_mbps: AtomicU64,
    window_start: Mutex<Instant>,
    window_bytes: AtomicU64,
}

impl AdaptiveSemaphore {
    pub fn new() -> Self {
        let initial = adaptive_max_concurrency().min(ADAPTIVE_INITIAL_CONCURRENCY);
        Self {
            semaphore: Semaphore::new(initial),
            notify: Notify::new(),
            target: AtomicUsize::new(initial),
            active: AtomicUsize::new(0),
            ewma_throughput_mbps: AtomicU64::new(0),
            best_throughput_mbps: AtomicU64::new(0),
            window_start: Mutex::new(Instant::now()),
            window_bytes: AtomicU64::new(0),
        }
    }

    pub async fn acquire(&self) -> AdaptivePermit<'_> {
        loop {
            let permit = match self.semaphore.acquire().await {
                Ok(p) => p,
                Err(_) => {
                    log::error!("adaptive semaphore closed unexpectedly");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
            };
            let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
            let target = self.target.load(Ordering::Acquire);
            if active <= target {
                return AdaptivePermit {
                    _permit: permit,
                    adaptive: self,
                };
            }
            self.active.fetch_sub(1, Ordering::AcqRel);
            drop(permit);
            self.notify.notified().await;
        }
    }

    pub fn record_bytes(&self, bytes: u64) {
        self.window_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub fn current_target(&self) -> usize {
        self.target.load(Ordering::Acquire)
    }

    #[allow(dead_code)]
    pub fn current_active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    fn dec_active(&self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
        self.notify.notify_one();
    }

    pub fn adjust(&self) -> usize {
        let mut window_start = self
            .window_start
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let now = Instant::now();
        let elapsed = now.duration_since(*window_start).as_secs_f64();

        if elapsed < ADAPTIVE_WINDOW_SECS as f64 {
            drop(window_start);
            return self.target.load(Ordering::Acquire);
        }

        let window_bytes = self.window_bytes.swap(0, Ordering::AcqRel);
        let throughput_bps = window_bytes as f64 / elapsed;
        let throughput_mbps = throughput_bps / 1_048_576.0;

        let prev_ewma_raw = self.ewma_throughput_mbps.load(Ordering::Acquire);
        let prev_ewma = prev_ewma_raw as f64 / THROUGHPUT_SCALE;
        let new_ewma = if prev_ewma == 0.0 {
            throughput_mbps
        } else {
            EWMA_ALPHA * throughput_mbps + (1.0 - EWMA_ALPHA) * prev_ewma
        };
        self.ewma_throughput_mbps
            .store((new_ewma * THROUGHPUT_SCALE) as u64, Ordering::Release);

        let best_raw = self.best_throughput_mbps.load(Ordering::Acquire);
        let best = if best_raw == 0 {
            0.0
        } else {
            let b = best_raw as f64 / THROUGHPUT_SCALE;
            b * BEST_DECAY
        };

        let effective_best = if new_ewma > best { new_ewma } else { best };
        self.best_throughput_mbps.store(
            (effective_best * THROUGHPUT_SCALE) as u64,
            Ordering::Release,
        );

        let current = self.target.load(Ordering::Acquire);
        let new_target = Self::calculate_new_target(current, new_ewma, prev_ewma, effective_best);

        if new_target > current {
            let delta = new_target - current;
            self.semaphore.add_permits(delta);
            self.notify.notify_waiters();
        } else if new_target < current {
            self.notify.notify_one();
        }

        self.target.store(new_target, Ordering::Release);
        *window_start = now;
        new_target
    }

    fn calculate_new_target(current: usize, ewma: f64, prev_ewma: f64, best: f64) -> usize {
        let max = adaptive_max_concurrency();
        if best > 0.0 && ewma >= best * 0.5 {
            if current < max * 3 / 4 {
                let increase = (current / 2).max(4);
                (current + increase).min(max)
            } else {
                let increase = (current / 8).max(2);
                (current + increase).min(max)
            }
        } else if best == 0.0 && prev_ewma == 0.0 && ewma > 0.0 {
            let increase = (current / 2).max(4);
            (current + increase).min(max)
        } else if prev_ewma > 0.0 && ewma >= prev_ewma * 0.8 {
            let increase = (current / 8).max(2);
            (current + increase).min(max)
        } else if prev_ewma > 0.0 && ewma < prev_ewma * 0.5 {
            let decreased = (current * 7) / 10;
            decreased.max(ADAPTIVE_MIN_CONCURRENCY)
        } else {
            current.saturating_sub(1).max(ADAPTIVE_MIN_CONCURRENCY)
        }
    }
}

impl Default for AdaptiveSemaphore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_semaphore_has_initial_target() {
        let sem = AdaptiveSemaphore::new();
        let expected = adaptive_max_concurrency().min(ADAPTIVE_INITIAL_CONCURRENCY);
        assert_eq!(sem.current_target(), expected);
        assert_eq!(sem.current_active(), 0);
    }

    #[test]
    fn calculate_new_target_ramp_up_near_max() {
        let max = super::adaptive_max_concurrency();
        let current = if max > 1 { (max * 3 / 4).max(1) } else { 1 };
        let increase = (current / 8).max(2);
        let expected = (current + increase).min(max);
        let target = AdaptiveSemaphore::calculate_new_target(current, 100.0, 80.0, 100.0);
        assert_eq!(target, expected);
    }

    #[test]
    fn calculate_new_target_ramp_up_aggressive() {
        let max = super::adaptive_max_concurrency();
        let current = if max > 2 { max / 2 } else { 1 };
        let increase = (current / 2).max(4);
        let expected = (current + increase).min(max);
        let target = AdaptiveSemaphore::calculate_new_target(current, 100.0, 80.0, 100.0);
        assert_eq!(target, expected);
    }

    #[test]
    fn calculate_new_target_stable_gentle_increase() {
        let max = super::adaptive_max_concurrency();
        let current = if max > 2 { max / 2 } else { 1 };
        let increase = (current / 8).max(2);
        let expected = (current + increase).min(max);
        let target = AdaptiveSemaphore::calculate_new_target(current, 45.0, 50.0, 100.0);
        assert_eq!(target, expected);
    }

    #[test]
    fn calculate_new_target_significant_drop() {
        let target = AdaptiveSemaphore::calculate_new_target(60, 20.0, 50.0, 100.0);
        assert_eq!(target, 42); // 60 * 7 / 10 = 42
    }

    #[test]
    fn calculate_new_target_moderate_drop() {
        let target = AdaptiveSemaphore::calculate_new_target(40, 38.0, 50.0, 100.0);
        assert_eq!(target, 39); // 40 - 1 = 39
    }

    #[test]
    fn calculate_new_target_respects_max() {
        let max = super::adaptive_max_concurrency();
        let target = AdaptiveSemaphore::calculate_new_target(max, 100.0, 80.0, 100.0);
        assert_eq!(target, max);
    }

    #[test]
    fn calculate_new_target_respects_min() {
        let target =
            AdaptiveSemaphore::calculate_new_target(ADAPTIVE_MIN_CONCURRENCY, 5.0, 50.0, 100.0);
        assert_eq!(target, ADAPTIVE_MIN_CONCURRENCY);
    }

    #[test]
    fn calculate_new_target_multiplicative_decrease_respects_min() {
        let target = AdaptiveSemaphore::calculate_new_target(10, 5.0, 50.0, 100.0);
        assert_eq!(target, ADAPTIVE_MIN_CONCURRENCY);
    }

    #[test]
    fn calculate_new_target_zero_best_explores() {
        let max = super::adaptive_max_concurrency();
        let current = if max > 1 { max / 2 } else { 1 };
        let target = AdaptiveSemaphore::calculate_new_target(current, 50.0, 0.0, 0.0);
        let increase = (current / 2).max(4);
        let expected = (current + increase).min(max);
        assert!(
            target > current,
            "expected {target} > {current} with max={max}"
        );
        assert_eq!(target, expected);
    }

    #[test]
    fn ewma_first_sample_is_raw_throughput() {
        let sem = AdaptiveSemaphore::new();
        assert_eq!(sem.ewma_throughput_mbps.load(Ordering::Relaxed), 0);

        sem.window_bytes
            .fetch_add(200 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem
                .window_start
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(super::ADAPTIVE_WINDOW_SECS);
        }
        sem.adjust();

        let ewma_raw = sem.ewma_throughput_mbps.load(Ordering::Relaxed);
        let ewma = ewma_raw as f64 / THROUGHPUT_SCALE;
        assert!(ewma > 180.0 && ewma < 210.0); // ~190.7 MiB/s: 200MB / 1s
    }

    #[test]
    fn ewma_smoothing_second_sample() {
        let sem = AdaptiveSemaphore::new();

        sem.window_bytes
            .fetch_add(200 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem
                .window_start
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(super::ADAPTIVE_WINDOW_SECS);
        }
        sem.adjust();

        sem.window_bytes
            .fetch_add(100 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem
                .window_start
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(super::ADAPTIVE_WINDOW_SECS);
        }
        sem.adjust();

        let ewma_raw = sem.ewma_throughput_mbps.load(Ordering::Relaxed);
        let ewma = ewma_raw as f64 / THROUGHPUT_SCALE;
        // First: ~190.7 MiB/s. Second: ~95.4 MiB/s.
        // EWMA = 0.25*95.4 + 0.75*190.7 ≈ 166.9 MiB/s
        assert!(ewma > 155.0 && ewma < 180.0);
    }

    #[test]
    fn best_throughput_decay() {
        let sem = AdaptiveSemaphore::new();

        sem.window_bytes
            .fetch_add(200 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem
                .window_start
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(super::ADAPTIVE_WINDOW_SECS);
        }
        sem.adjust();
        let best_after_first =
            sem.best_throughput_mbps.load(Ordering::Relaxed) as f64 / THROUGHPUT_SCALE;

        sem.window_bytes.fetch_add(1, Ordering::Relaxed);
        {
            let mut ws = sem
                .window_start
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(super::ADAPTIVE_WINDOW_SECS);
        }
        sem.adjust();
        let best_after_second =
            sem.best_throughput_mbps.load(Ordering::Relaxed) as f64 / THROUGHPUT_SCALE;

        assert!(best_after_second < best_after_first);
    }

    #[tokio::test]
    async fn acquire_increments_active() {
        let sem = AdaptiveSemaphore::new();
        assert_eq!(sem.current_active(), 0);
        let _permit = sem.acquire().await;
        assert_eq!(sem.current_active(), 1);
    }

    #[tokio::test]
    async fn drop_permit_decrements_active() {
        let sem = AdaptiveSemaphore::new();
        {
            let _permit = sem.acquire().await;
            assert_eq!(sem.current_active(), 1);
        }
        assert_eq!(sem.current_active(), 0);
    }

    #[tokio::test]
    async fn acquire_releases_permits_on_drop() {
        let sem = AdaptiveSemaphore::new();
        let initial_target = sem.current_target();
        for _ in 0..initial_target {
            let _permit = sem.acquire().await;
        }
        // All permits used, next acquire would block
        // Drop one to free a permit
        drop(sem.acquire().await);
        // Now one permit is available again
        let _permit = sem.acquire().await;
    }

    #[test]
    fn record_bytes_accumulates() {
        let sem = AdaptiveSemaphore::new();
        sem.record_bytes(100);
        sem.record_bytes(200);
        assert_eq!(sem.window_bytes.load(Ordering::Relaxed), 300);
    }

    #[test]
    fn adjust_respects_window_timing() {
        let sem = AdaptiveSemaphore::new();
        // Window hasn't elapsed, so adjust should return current target unchanged
        let result = sem.adjust();
        let expected = adaptive_max_concurrency().min(ADAPTIVE_INITIAL_CONCURRENCY);
        assert_eq!(result, expected);
    }

    #[test]
    fn ewma_convergence_after_repeated_identical_throughput() {
        let sem = AdaptiveSemaphore::new();
        let bytes_per_window = 200 * 1024 * 1024;
        let window_duration = std::time::Duration::from_secs(super::ADAPTIVE_WINDOW_SECS);
        for _ in 0..5 {
            sem.window_bytes
                .fetch_add(bytes_per_window, Ordering::Relaxed);
            {
                let mut ws = sem
                    .window_start
                    .lock()
                    .unwrap_or_else(|err| err.into_inner());
                *ws = Instant::now() - window_duration;
            }
            sem.adjust();
        }
        let ewma_raw = sem.ewma_throughput_mbps.load(Ordering::Relaxed);
        let ewma = ewma_raw as f64 / THROUGHPUT_SCALE;
        let expected = bytes_per_window as f64 / window_duration.as_secs_f64() / 1_048_576.0;
        assert!((ewma - expected).abs() / expected < 0.05);
    }

    #[test]
    fn aimd_increase_at_exactly_80_percent_of_best() {
        let max = super::adaptive_max_concurrency();
        let best = 100.0;
        let ewma = best * 0.6;
        let prev_ewma = 90.0;
        let current = if max > 1 { max / 2 } else { 1 };
        let target = AdaptiveSemaphore::calculate_new_target(current, ewma, prev_ewma, best);
        assert!(
            target > current,
            "expected {target} > {current} with max={max}"
        );
    }

    #[test]
    fn aimd_no_multiplicative_decrease_at_exactly_70_percent() {
        let prev_ewma = 100.0;
        let ewma = prev_ewma * 0.5;
        let best = 120.0;
        let current = 40;
        let target = AdaptiveSemaphore::calculate_new_target(current, ewma, prev_ewma, best);
        assert_eq!(target, 39);
    }

    #[test]
    fn calculate_new_target_at_exact_max_boundary() {
        let target = AdaptiveSemaphore::calculate_new_target(
            super::adaptive_max_concurrency(),
            100.0,
            80.0,
            100.0,
        );
        assert_eq!(target, super::adaptive_max_concurrency());
    }

    #[tokio::test]
    async fn concurrent_acquire_release() {
        let sem = AdaptiveSemaphore::new();
        let sem = std::sync::Arc::new(sem);
        let mut handles = Vec::new();
        for _ in 0..10 {
            let s = sem.clone();
            handles.push(tokio::spawn(async move {
                let permit = s.acquire().await;
                drop(permit);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(sem.current_active(), 0);
    }

    #[test]
    fn adjust_increases_semaphore_permits() {
        let sem = AdaptiveSemaphore::new();
        sem.window_bytes
            .fetch_add(200 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem
                .window_start
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(super::ADAPTIVE_WINDOW_SECS);
        }
        let old_target = sem.current_target();
        let new_target = sem.adjust();
        if new_target > old_target {
            let extra = new_target - old_target;
            for _ in 0..extra {
                let _permit = sem
                    .semaphore
                    .try_acquire()
                    .expect("extra permit should be available");
            }
        }
    }

    #[test]
    fn record_bytes_window_isolation() {
        let sem = AdaptiveSemaphore::new();
        sem.record_bytes(500);
        sem.window_bytes
            .fetch_add(200 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem
                .window_start
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(super::ADAPTIVE_WINDOW_SECS);
        }
        sem.adjust();
        let window_bytes_after = sem.window_bytes.load(Ordering::Relaxed);
        assert_eq!(window_bytes_after, 0);
        sem.record_bytes(300);
        assert_eq!(sem.window_bytes.load(Ordering::Relaxed), 300);
    }

    #[test]
    fn multiple_rapid_adjust_calls() {
        let sem = AdaptiveSemaphore::new();
        sem.window_bytes
            .fetch_add(200 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem
                .window_start
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(super::ADAPTIVE_WINDOW_SECS);
        }
        let first = sem.adjust();
        let second = sem.adjust();
        assert_eq!(first, second);
    }

    #[test]
    fn best_throughput_updates_on_new_high() {
        let sem = AdaptiveSemaphore::new();
        sem.window_bytes
            .fetch_add(200 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem
                .window_start
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(super::ADAPTIVE_WINDOW_SECS);
        }
        sem.adjust();
        let best_after_low =
            sem.best_throughput_mbps.load(Ordering::Relaxed) as f64 / THROUGHPUT_SCALE;
        sem.window_bytes
            .fetch_add(400 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem
                .window_start
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(super::ADAPTIVE_WINDOW_SECS);
        }
        sem.adjust();
        let best_after_high =
            sem.best_throughput_mbps.load(Ordering::Relaxed) as f64 / THROUGHPUT_SCALE;
        assert!(best_after_high > best_after_low);
    }
}
