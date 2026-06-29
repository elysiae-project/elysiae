use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;
use tauri_plugin_log::log;

pub struct PipelineProfiler {
    #[cfg(feature = "pipeline-profiling")]
    start: Instant,
    #[cfg(feature = "pipeline-profiling")]
    window_bytes: AtomicU64,
    #[cfg(feature = "pipeline-profiling")]
    window_chunks: AtomicU64,
    #[cfg(feature = "pipeline-profiling")]
    window_start_nanos: AtomicU64,
    pub download_ns: AtomicU64,
    pub download_count: AtomicU64,
    pub verify_ns: AtomicU64,
    pub verify_count: AtomicU64,
    pub post_download_ns: AtomicU64,
    pub post_download_count: AtomicU64,
    #[cfg(feature = "pipeline-profiling")]
    active_downloads: AtomicUsize,
    #[cfg(feature = "pipeline-profiling")]
    peak_active_downloads: AtomicUsize,
    #[cfg(feature = "pipeline-profiling")]
    idle_ns: AtomicU64,
    #[cfg(feature = "pipeline-profiling")]
    window_idle_ns: AtomicU64,
    pub assembly_decompress_ns: AtomicU64,
    pub assembly_decompress_count: AtomicU64,
    pub assembly_write_ns: AtomicU64,
    pub assembly_write_count: AtomicU64,
    pub assembly_total_ns: AtomicU64,
    pub assembly_total_count: AtomicU64,
    #[cfg(feature = "pipeline-profiling")]
    total_bytes_downloaded: AtomicU64,
    #[cfg(feature = "pipeline-profiling")]
    report_count: AtomicU64,
    pub total_chunks: AtomicUsize,
}

impl PipelineProfiler {
    pub fn new() -> Self {
        #[cfg(feature = "pipeline-profiling")]
        let now = {
            static EPOCH: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
            EPOCH.elapsed().as_nanos() as u64
        };
        Self {
            #[cfg(feature = "pipeline-profiling")]
            start: Instant::now(),
            #[cfg(feature = "pipeline-profiling")]
            window_bytes: AtomicU64::new(0),
            #[cfg(feature = "pipeline-profiling")]
            window_chunks: AtomicU64::new(0),
            #[cfg(feature = "pipeline-profiling")]
            window_start_nanos: AtomicU64::new(now),
            download_ns: AtomicU64::new(0),
            download_count: AtomicU64::new(0),
            verify_ns: AtomicU64::new(0),
            verify_count: AtomicU64::new(0),
            post_download_ns: AtomicU64::new(0),
            post_download_count: AtomicU64::new(0),
            #[cfg(feature = "pipeline-profiling")]
            active_downloads: AtomicUsize::new(0),
            #[cfg(feature = "pipeline-profiling")]
            peak_active_downloads: AtomicUsize::new(0),
            #[cfg(feature = "pipeline-profiling")]
            idle_ns: AtomicU64::new(0),
            #[cfg(feature = "pipeline-profiling")]
            window_idle_ns: AtomicU64::new(0),
            assembly_decompress_ns: AtomicU64::new(0),
            assembly_decompress_count: AtomicU64::new(0),
            assembly_write_ns: AtomicU64::new(0),
            assembly_write_count: AtomicU64::new(0),
            assembly_total_ns: AtomicU64::new(0),
            assembly_total_count: AtomicU64::new(0),
            #[cfg(feature = "pipeline-profiling")]
            total_bytes_downloaded: AtomicU64::new(0),
            #[cfg(feature = "pipeline-profiling")]
            report_count: AtomicU64::new(0),
            total_chunks: AtomicUsize::new(0),
        }
    }

    #[inline]
    pub fn download_enter(&self) {
        #[cfg(feature = "pipeline-profiling")]
        {
            let prev = self.active_downloads.fetch_add(1, Ordering::Relaxed);
            self.peak_active_downloads
                .fetch_max(prev + 1, Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn download_exit(&self) {
        #[cfg(feature = "pipeline-profiling")]
        {
            self.active_downloads.fetch_sub(1, Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn record_idle(&self, _ns: u64) {
        #[cfg(feature = "pipeline-profiling")]
        {
            self.idle_ns.fetch_add(_ns, Ordering::Relaxed);
            self.window_idle_ns.fetch_add(_ns, Ordering::Relaxed);
        }
    }

    pub fn report(&self) {
        #[cfg(not(feature = "pipeline-profiling"))]
        {}

        #[cfg(feature = "pipeline-profiling")]
        {
            let count = self.report_count.fetch_add(1, Ordering::Relaxed) + 1;
            let elapsed = self.start.elapsed().as_secs_f64();
            if elapsed < 1.0 {
                return;
            }

            let total_chunks = self.download_count.load(Ordering::Relaxed);
            if total_chunks == 0 {
                return;
            }

            static EPOCH: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
            let now = EPOCH.elapsed().as_nanos() as u64;
            let window_start = self.window_start_nanos.swap(now, Ordering::Relaxed);
            let window_elapsed_ns = now.saturating_sub(window_start);
            let window_elapsed_s = window_elapsed_ns as f64 / 1_000_000_000.0;

            let window_bytes = self.window_bytes.swap(0, Ordering::Relaxed);
            let _window_chunks = self.window_chunks.swap(0, Ordering::Relaxed);
            let window_idle = self.window_idle_ns.swap(0, Ordering::Relaxed);

            let window_throughput_mibs = if window_elapsed_s > 0.0 {
                window_bytes as f64 / window_elapsed_s / 1_048_576.0
            } else {
                0.0
            };

            let cumulative_throughput_mibs = {
                let total_bytes = self.total_bytes_downloaded.load(Ordering::Relaxed);
                total_bytes as f64 / elapsed / 1_048_576.0
            };

            let active = self.active_downloads.load(Ordering::Relaxed);
            let peak_active = self.peak_active_downloads.load(Ordering::Relaxed);

            let avg_download_us = if self.download_count.load(Ordering::Relaxed) > 0 {
                self.download_ns.load(Ordering::Relaxed) as f64
                    / self.download_count.load(Ordering::Relaxed) as f64
                    / 1000.0
            } else {
                0.0
            };

            let avg_verify_us = if self.verify_count.load(Ordering::Relaxed) > 0 {
                self.verify_ns.load(Ordering::Relaxed) as f64
                    / self.verify_count.load(Ordering::Relaxed) as f64
                    / 1000.0
            } else {
                0.0
            };

            let avg_post_us = if self.post_download_count.load(Ordering::Relaxed) > 0 {
                self.post_download_ns.load(Ordering::Relaxed) as f64
                    / self.post_download_count.load(Ordering::Relaxed) as f64
                    / 1000.0
            } else {
                0.0
            };

            let avg_chunk_us = avg_download_us + avg_verify_us + avg_post_us;
            let download_pct = if avg_chunk_us > 0.0 {
                avg_download_us / avg_chunk_us * 100.0
            } else {
                0.0
            };

            let avg_assembly_total_us = if self.assembly_total_count.load(Ordering::Relaxed) > 0 {
                self.assembly_total_ns.load(Ordering::Relaxed) as f64
                    / self.assembly_total_count.load(Ordering::Relaxed) as f64
                    / 1000.0
            } else {
                0.0
            };

            let idle_s = window_idle as f64 / 1_000_000_000.0;
            let total_worker_s =
                self.total_chunks.load(Ordering::Relaxed) as f64 * avg_chunk_us / 1_000_000.0;
            let total_available_s = elapsed * 64.0;
            let utilization_pct = if total_available_s > 0.0 {
                (total_worker_s / total_available_s * 100.0).min(100.0)
            } else {
                0.0
            };

            let remaining = self
                .total_chunks
                .load(Ordering::Relaxed)
                .saturating_sub(total_chunks as usize);

            log::info!(
                "[PROFILE #{count}] elapsed={elapsed:.1}s chunks={total_chunks} remaining={remaining} \
                 window_throughput={window_throughput_mibs:.1}MiB/s cumulative_throughput={cumulative_throughput_mibs:.1}MiB/s"
            );
            log::info!(
                "[PROFILE #{count}] active={active}/{peak_active}peak workers=64 \
                 utilization={utilization_pct:.0}% idle={idle_s:.2}s"
            );
            log::info!(
                "[PROFILE #{count}] per_chunk: download={avg_download_us:.0}us({download_pct:.0}%) \
                 verify={avg_verify_us:.0}us post={avg_post_us:.0}us"
            );
            log::info!("[PROFILE #{count}] assembly_avg={avg_assembly_total_us:.0}us");

            fn process_rss_mb() -> Option<f64> {
                let data = std::fs::read_to_string("/proc/self/statm").ok()?;
                let resident_pages: u64 = data.split_whitespace().nth(1)?.parse().ok()?;
                let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 };
                Some(resident_pages as f64 * page_size as f64 / 1_048_576.0)
            }

            fn jemalloc_stats_mb() -> Option<(f64, f64, f64, f64, f64)> {
                #[cfg(not(target_env = "msvc"))]
                {
                    let allocated: usize = tikv_jemalloc_ctl::stats::allocated::read().ok()?;
                    let active: usize = tikv_jemalloc_ctl::stats::active::read().ok()?;
                    let resident: usize = tikv_jemalloc_ctl::stats::resident::read().ok()?;
                    let mapped: usize = tikv_jemalloc_ctl::stats::mapped::read().ok()?;
                    let retained: usize = tikv_jemalloc_ctl::stats::retained::read().ok()?;
                    let mb = |v: usize| v as f64 / 1_048_576.0;
                    Some((
                        mb(allocated),
                        mb(active),
                        mb(resident),
                        mb(mapped),
                        mb(retained),
                    ))
                }
                #[cfg(target_env = "msvc")]
                None
            }

            if let Some(rss) = process_rss_mb() {
                if let Some((allocated, active, resident, mapped, retained)) = jemalloc_stats_mb() {
                    let non_jemalloc = (rss - resident).max(0.0);
                    log::info!(
                        "[PROFILE #{count}] rss={rss:.0}MB jemalloc: allocated={allocated:.0}MB active={active:.0}MB resident={resident:.0}MB mapped={mapped:.0}MB retained={retained:.0}MB non_jemalloc={non_jemalloc:.0}MB"
                    );
                } else {
                    log::info!("[PROFILE #{count}] rss={rss:.0}MB");
                }
            }
        }
    }
}

pub struct ChunkTimer<'a> {
    profiler: &'a PipelineProfiler,
    #[cfg(feature = "pipeline-profiling")]
    phase_start: Instant,
}

impl<'a> ChunkTimer<'a> {
    pub fn new(profiler: &'a PipelineProfiler) -> Self {
        profiler.download_enter();
        Self {
            profiler,
            #[cfg(feature = "pipeline-profiling")]
            phase_start: Instant::now(),
        }
    }

    #[inline]
    pub fn record_phase(&mut self, phase: ChunkPhase) {
        #[cfg(feature = "pipeline-profiling")]
        {
            let now = Instant::now();
            let elapsed = now.duration_since(self.phase_start);
            let ns = elapsed.as_nanos() as u64;

            match phase {
                ChunkPhase::Verify => {
                    self.profiler.verify_ns.fetch_add(ns, Ordering::Relaxed);
                    self.profiler.verify_count.fetch_add(1, Ordering::Relaxed);
                }
                ChunkPhase::Download => {
                    self.profiler.download_ns.fetch_add(ns, Ordering::Relaxed);
                    self.profiler.download_count.fetch_add(1, Ordering::Relaxed);
                }
                ChunkPhase::PostDownload => {
                    self.profiler
                        .post_download_ns
                        .fetch_add(ns, Ordering::Relaxed);
                    self.profiler
                        .post_download_count
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            self.phase_start = now;
        }
    }

    #[inline]
    pub fn finish(self, chunk_size: u64, was_downloaded: bool) {
        self.profiler.download_exit();
        #[cfg(feature = "pipeline-profiling")]
        {
            if was_downloaded {
                self.profiler
                    .window_bytes
                    .fetch_add(chunk_size, Ordering::Relaxed);
                self.profiler
                    .total_bytes_downloaded
                    .fetch_add(chunk_size, Ordering::Relaxed);
            }
            self.profiler.window_chunks.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub enum ChunkPhase {
    Verify,
    Download,
    PostDownload,
}

pub struct AssemblyTimer<'a> {
    profiler: &'a PipelineProfiler,
    #[cfg(feature = "pipeline-profiling")]
    start: Instant,
}

impl<'a> AssemblyTimer<'a> {
    pub fn new(profiler: &'a PipelineProfiler) -> Self {
        Self {
            profiler,
            #[cfg(feature = "pipeline-profiling")]
            start: Instant::now(),
        }
    }

    #[allow(dead_code)]
    pub fn record_decompress_time(&self, duration: std::time::Duration) {
        let ns = duration.as_nanos() as u64;
        self.profiler
            .assembly_decompress_ns
            .fetch_add(ns, Ordering::Relaxed);
        self.profiler
            .assembly_decompress_count
            .fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub fn record_write_time(&self, duration: std::time::Duration) {
        let ns = duration.as_nanos() as u64;
        self.profiler
            .assembly_write_ns
            .fetch_add(ns, Ordering::Relaxed);
        self.profiler
            .assembly_write_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn finish(self) {
        #[cfg(feature = "pipeline-profiling")]
        {
            let elapsed = self.start.elapsed().as_nanos() as u64;
            self.profiler
                .assembly_total_ns
                .fetch_add(elapsed, Ordering::Relaxed);
            self.profiler
                .assembly_total_count
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}
