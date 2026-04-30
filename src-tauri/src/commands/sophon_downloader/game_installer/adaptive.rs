use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use tokio::sync::{Semaphore, SemaphorePermit};

use super::*;

const EWMA_ALPHA: f64 = 0.3;
const THROUGHPUT_SCALE: f64 = 1000.0;
const BEST_DECAY: f64 = 0.95;

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
    target: AtomicUsize,
    active: AtomicUsize,
    total_bytes: AtomicU64,
    ewma_throughput_mbps: AtomicU64,
    best_throughput_mbps: AtomicU64,
    window_start: Mutex<Instant>,
    window_bytes: AtomicU64,
}

impl AdaptiveSemaphore {
    pub fn new() -> Self {
        Self {
            semaphore: Semaphore::new(ADAPTIVE_INITIAL_CONCURRENCY),
            target: AtomicUsize::new(ADAPTIVE_INITIAL_CONCURRENCY),
            active: AtomicUsize::new(0),
            total_bytes: AtomicU64::new(0),
            ewma_throughput_mbps: AtomicU64::new(0),
            best_throughput_mbps: AtomicU64::new(0),
            window_start: Mutex::new(Instant::now()),
            window_bytes: AtomicU64::new(0),
        }
    }

    pub async fn acquire(&self) -> AdaptivePermit<'_> {
        let permit = self
            .semaphore
            .acquire()
            .await
            .expect("adaptive semaphore closed unexpectedly");
        self.active.fetch_add(1, Ordering::AcqRel);
        AdaptivePermit {
            _permit: permit,
            adaptive: self,
        }
    }

    pub fn record_bytes(&self, bytes: u64) {
        self.total_bytes.fetch_add(bytes, Ordering::Relaxed);
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
    }

    pub fn adjust(&self) -> usize {
        let mut window_start = self.window_start.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let elapsed = now.duration_since(*window_start).as_secs_f64();

        if elapsed < ADAPTIVE_WINDOW_SECS as f64 {
            drop(window_start);
            return self.target.load(Ordering::Acquire);
        }

        let window_bytes = self.window_bytes.swap(0, Ordering::AcqRel);
        let throughput_bps = window_bytes as f64 / elapsed;
        let throughput_mbps = throughput_bps / 1_048_576.0;

        let prev_ewma_raw = self.ewma_throughput_mbps.load(Ordering::Relaxed);
        let prev_ewma = prev_ewma_raw as f64 / THROUGHPUT_SCALE;
        let new_ewma = if prev_ewma == 0.0 {
            throughput_mbps
        } else {
            EWMA_ALPHA * throughput_mbps + (1.0 - EWMA_ALPHA) * prev_ewma
        };
        self.ewma_throughput_mbps
            .store((new_ewma * THROUGHPUT_SCALE) as u64, Ordering::Relaxed);

        let best_raw = self.best_throughput_mbps.load(Ordering::Relaxed);
        let best = if best_raw == 0 {
            0.0
        } else {
            let b = best_raw as f64 / THROUGHPUT_SCALE;
            b * BEST_DECAY
        };

        let effective_best = if new_ewma > best { new_ewma } else { best };
        self.best_throughput_mbps.store(
            (effective_best * THROUGHPUT_SCALE) as u64,
            Ordering::Relaxed,
        );

        let current = self.target.load(Ordering::Acquire);
        let new_target = Self::calculate_new_target(current, new_ewma, prev_ewma, effective_best);

        if new_target > current {
            let delta = new_target - current;
            self.semaphore.add_permits(delta);
        } else if new_target < current {
            let delta = current - new_target;
            self.remove_permits(delta);
        }

        self.target.store(new_target, Ordering::Release);
        *window_start = now;
        new_target
    }

    fn remove_permits(&self, count: usize) {
        for _ in 0..count {
            match self.semaphore.try_acquire() {
                Ok(permit) => permit.forget(),
                Err(_) => break,
            }
        }
    }

    fn calculate_new_target(current: usize, ewma: f64, prev_ewma: f64, best: f64) -> usize {
        if best > 0.0 && ewma >= best * 0.8 {
            if current < ADAPTIVE_MAX_CONCURRENCY * 3 / 4 {
                let increase = (current / 4).max(4);
                (current + increase).min(ADAPTIVE_MAX_CONCURRENCY)
            } else {
                let increase = (current / 16).max(1);
                (current + increase).min(ADAPTIVE_MAX_CONCURRENCY)
            }
        } else if best == 0.0 && prev_ewma == 0.0 && ewma > 0.0 {
            let increase = (current / 2).max(4);
            (current + increase).min(ADAPTIVE_MAX_CONCURRENCY)
        } else if prev_ewma > 0.0 && ewma >= prev_ewma * 0.9 {
            let increase = (current / 16).max(1);
            (current + increase).min(ADAPTIVE_MAX_CONCURRENCY)
        } else if prev_ewma > 0.0 && ewma < prev_ewma * 0.7 {
            let decreased = (current * 7) / 10;
            decreased.max(ADAPTIVE_MIN_CONCURRENCY)
        } else {
            current.saturating_sub(2).max(ADAPTIVE_MIN_CONCURRENCY)
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
        assert_eq!(sem.current_target(), ADAPTIVE_INITIAL_CONCURRENCY);
        assert_eq!(sem.current_active(), 0);
    }

    #[test]
    fn calculate_new_target_ramp_up_aggressive() {
        let target = AdaptiveSemaphore::calculate_new_target(16, 100.0, 80.0, 100.0);
        assert_eq!(target, 20); // 16 + max(4, 4) = 20
    }

    #[test]
    fn calculate_new_target_ramp_up_near_max() {
        let target = AdaptiveSemaphore::calculate_new_target(56, 100.0, 80.0, 100.0);
        assert_eq!(target, 59); // 56 >= 48 (3/4 of 64), gentle: 56 + max(1, 3) = 59
    }

    #[test]
    fn calculate_new_target_stable_gentle_increase() {
        let target = AdaptiveSemaphore::calculate_new_target(20, 45.0, 50.0, 100.0);
        // ewma (45) >= prev_ewma * 0.9 (45) → stable, gentle increase
        assert_eq!(target, 21); // 20 + max(1, 1) = 21
    }

    #[test]
    fn calculate_new_target_significant_drop() {
        let target = AdaptiveSemaphore::calculate_new_target(30, 20.0, 50.0, 100.0);
        // ewma (20) < prev_ewma * 0.7 (35) → multiplicative decrease
        assert_eq!(target, 21); // 30 * 7 / 10 = 21
    }

    #[test]
    fn calculate_new_target_moderate_drop() {
        let target = AdaptiveSemaphore::calculate_new_target(20, 38.0, 50.0, 100.0);
        // ewma (38) < prev * 0.9 (45) but >= prev * 0.7 (35) → moderate drop
        assert_eq!(target, 18); // 20 - 2 = 18
    }

    #[test]
    fn calculate_new_target_respects_max() {
        let target = AdaptiveSemaphore::calculate_new_target(63, 100.0, 80.0, 100.0);
        assert_eq!(target, ADAPTIVE_MAX_CONCURRENCY); // capped at 64
    }

    #[test]
    fn calculate_new_target_respects_min() {
        let target =
            AdaptiveSemaphore::calculate_new_target(ADAPTIVE_MIN_CONCURRENCY, 5.0, 50.0, 100.0);
        assert_eq!(target, ADAPTIVE_MIN_CONCURRENCY); // floor at 8
    }

    #[test]
    fn calculate_new_target_multiplicative_decrease_respects_min() {
        let target = AdaptiveSemaphore::calculate_new_target(10, 5.0, 50.0, 100.0);
        // 10 * 7 / 10 = 7, floored to MIN (8)
        assert_eq!(target, ADAPTIVE_MIN_CONCURRENCY);
    }

    #[test]
    fn calculate_new_target_zero_best_explores() {
        let target = AdaptiveSemaphore::calculate_new_target(16, 50.0, 0.0, 0.0);
        // best=0, so ewma >= 0.8*best (0) is true → ramp-up
        assert!(target > 16);
    }

    #[test]
    fn ewma_first_sample_is_raw_throughput() {
        let sem = AdaptiveSemaphore::new();
        assert_eq!(sem.ewma_throughput_mbps.load(Ordering::Relaxed), 0);

        sem.window_bytes
            .fetch_add(200 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem.window_start.lock().unwrap_or_else(|e| e.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(2);
        }
        sem.adjust();

        let ewma_raw = sem.ewma_throughput_mbps.load(Ordering::Relaxed);
        let ewma = ewma_raw as f64 / THROUGHPUT_SCALE;
        assert!(ewma > 90.0 && ewma < 110.0); // ~100 MiB/s
    }

    #[test]
    fn ewma_smoothing_second_sample() {
        let sem = AdaptiveSemaphore::new();

        sem.window_bytes
            .fetch_add(200 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem.window_start.lock().unwrap_or_else(|e| e.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(2);
        }
        sem.adjust();

        sem.window_bytes
            .fetch_add(100 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem.window_start.lock().unwrap_or_else(|e| e.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(2);
        }
        sem.adjust();

        let ewma_raw = sem.ewma_throughput_mbps.load(Ordering::Relaxed);
        let ewma = ewma_raw as f64 / THROUGHPUT_SCALE;
        // First: 100 MiB/s. Second: 50 MiB/s. EWMA = 0.3*50 + 0.7*100 = 85
        assert!(ewma > 80.0 && ewma < 90.0);
    }

    #[test]
    fn best_throughput_decay() {
        let sem = AdaptiveSemaphore::new();

        sem.window_bytes
            .fetch_add(200 * 1024 * 1024, Ordering::Relaxed);
        {
            let mut ws = sem.window_start.lock().unwrap_or_else(|e| e.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(2);
        }
        sem.adjust();
        let best_after_first =
            sem.best_throughput_mbps.load(Ordering::Relaxed) as f64 / THROUGHPUT_SCALE;

        sem.window_bytes.fetch_add(1, Ordering::Relaxed);
        {
            let mut ws = sem.window_start.lock().unwrap_or_else(|e| e.into_inner());
            *ws = Instant::now() - std::time::Duration::from_secs(2);
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
        assert_eq!(sem.total_bytes.load(Ordering::Relaxed), 300);
        assert_eq!(sem.window_bytes.load(Ordering::Relaxed), 300);
    }

    #[test]
    fn adjust_respects_window_timing() {
        let sem = AdaptiveSemaphore::new();
        // Window hasn't elapsed, so adjust should return current target unchanged
        let result = sem.adjust();
        assert_eq!(result, ADAPTIVE_INITIAL_CONCURRENCY);
    }
}
