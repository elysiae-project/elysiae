use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use super::constants::*;

pub struct ActiveGuard<'a> {
    adaptive: &'a AdaptiveConcurrency,
}

impl<'a> ActiveGuard<'a> {
    pub fn new(adaptive: &'a AdaptiveConcurrency) -> Self {
        adaptive.inc_active();
        Self { adaptive }
    }
}

impl<'a> Drop for ActiveGuard<'a> {
    fn drop(&mut self) {
        self.adaptive.dec_active();
    }
}

pub struct AdaptiveConcurrency {
    target: AtomicUsize,
    active: AtomicUsize,
    total_bytes: AtomicU64,
    window_start: Mutex<Instant>,
    window_start_bytes: AtomicU64,
}

impl AdaptiveConcurrency {
    pub fn new() -> Self {
        Self {
            target: AtomicUsize::new(ADAPTIVE_INITIAL_CONCURRENCY),
            active: AtomicUsize::new(0),
            total_bytes: AtomicU64::new(0),
            window_start: Mutex::new(Instant::now()),
            window_start_bytes: AtomicU64::new(0),
        }
    }

    pub fn record_bytes(&self, bytes: u64) {
        self.total_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn can_start(&self) -> bool {
        self.active.load(Ordering::Acquire) < self.target.load(Ordering::Acquire)
    }

    fn inc_active(&self) {
        self.active.fetch_add(1, Ordering::AcqRel);
    }

    fn dec_active(&self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }

    pub fn adjust(&self) -> usize {
        let mut window_start = self.window_start.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(*window_start).as_secs_f64();
        let current = self.target.load(Ordering::Acquire);

        if elapsed < ADAPTIVE_WINDOW_SECS as f64 {
            drop(window_start);
            return current;
        }

        let total = self.total_bytes.load(Ordering::Relaxed);
        let start_bytes = self.window_start_bytes.load(Ordering::Relaxed);
        let bytes_this_window = total.saturating_sub(start_bytes);
        let throughput_bps = bytes_this_window as f64 / elapsed;
        let throughput_mbps = throughput_bps / 1_048_576.0;

        let new_limit = Self::calculate_new_limit(current, throughput_mbps);

        *window_start = now;
        self.window_start_bytes.store(total, Ordering::Relaxed);
        self.target.store(new_limit, Ordering::Release);
        new_limit
    }

    fn calculate_new_limit(current: usize, throughput_mbps: f64) -> usize {
        if throughput_mbps > 100.0 {
            (current + 4).min(ADAPTIVE_MAX_CONCURRENCY)
        } else if throughput_mbps > 50.0 {
            (current + 2).min(ADAPTIVE_MAX_CONCURRENCY)
        } else if throughput_mbps > 20.0 {
            current
        } else if throughput_mbps > 10.0 {
            current.saturating_sub(1).max(ADAPTIVE_MIN_CONCURRENCY)
        } else {
            current.saturating_sub(2).max(ADAPTIVE_MIN_CONCURRENCY)
        }
    }
}

impl Default for AdaptiveConcurrency {
    fn default() -> Self {
        Self::new()
    }
}
