use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri_plugin_log::log;

pub struct PipelineProfiler {
    start: Instant,

    // Download phase
    pub semaphore_wait_ns: AtomicU64,
    pub semaphore_wait_count: AtomicU64,
    pub download_wait_ns: AtomicU64, // HTTP request + response time
    pub download_wait_count: AtomicU64,
    pub stream_read_ns: AtomicU64, // actual body streaming time
    pub stream_read_count: AtomicU64,
    pub verify_ns: AtomicU64, // check_needs_download verification
    pub verify_count: AtomicU64,
    pub chunk_total_ns: AtomicU64, // end-to-end per chunk
    pub chunk_total_count: AtomicU64,

    // Assembly phase
    pub assembly_decompress_ns: AtomicU64,
    pub assembly_decompress_count: AtomicU64,
    pub assembly_write_ns: AtomicU64,
    pub assembly_write_count: AtomicU64,
    pub assembly_total_ns: AtomicU64,
    pub assembly_total_count: AtomicU64,

    pub total_bytes_downloaded: AtomicU64,
    pub report_count: AtomicU64,
}

impl PipelineProfiler {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            semaphore_wait_ns: AtomicU64::new(0),
            semaphore_wait_count: AtomicU64::new(0),
            download_wait_ns: AtomicU64::new(0),
            download_wait_count: AtomicU64::new(0),
            stream_read_ns: AtomicU64::new(0),
            stream_read_count: AtomicU64::new(0),
            verify_ns: AtomicU64::new(0),
            verify_count: AtomicU64::new(0),
            chunk_total_ns: AtomicU64::new(0),
            chunk_total_count: AtomicU64::new(0),
            assembly_decompress_ns: AtomicU64::new(0),
            assembly_decompress_count: AtomicU64::new(0),
            assembly_write_ns: AtomicU64::new(0),
            assembly_write_count: AtomicU64::new(0),
            assembly_total_ns: AtomicU64::new(0),
            assembly_total_count: AtomicU64::new(0),
            total_bytes_downloaded: AtomicU64::new(0),
            report_count: AtomicU64::new(0),
        }
    }

    pub fn report(&self) {
        let count = self.report_count.fetch_add(1, Ordering::Relaxed) + 1;
        let elapsed = self.start.elapsed().as_secs_f64();
        if elapsed < 0.5 {
            return;
        }

        let chunks = self.chunk_total_count.load(Ordering::Relaxed);
        if chunks == 0 {
            return;
        }

        let total_bytes = self.total_bytes_downloaded.load(Ordering::Relaxed);
        let throughput_mbps = total_bytes as f64 / elapsed / 1_048_576.0;

        let avg_semaphore_us = if self.semaphore_wait_count.load(Ordering::Relaxed) > 0 {
            self.semaphore_wait_ns.load(Ordering::Relaxed) as f64
                / self.semaphore_wait_count.load(Ordering::Relaxed) as f64
                / 1000.0
        } else {
            0.0
        };

        let avg_download_us = if self.download_wait_count.load(Ordering::Relaxed) > 0 {
            self.download_wait_ns.load(Ordering::Relaxed) as f64
                / self.download_wait_count.load(Ordering::Relaxed) as f64
                / 1000.0
        } else {
            0.0
        };

        let avg_stream_us = if self.stream_read_count.load(Ordering::Relaxed) > 0 {
            self.stream_read_ns.load(Ordering::Relaxed) as f64
                / self.stream_read_count.load(Ordering::Relaxed) as f64
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

        let avg_chunk_us = if chunks > 0 {
            self.chunk_total_ns.load(Ordering::Relaxed) as f64 / chunks as f64 / 1000.0
        } else {
            0.0
        };

        let avg_assembly_decompress_us =
            if self.assembly_decompress_count.load(Ordering::Relaxed) > 0 {
                self.assembly_decompress_ns.load(Ordering::Relaxed) as f64
                    / self.assembly_decompress_count.load(Ordering::Relaxed) as f64
                    / 1000.0
            } else {
                0.0
            };

        let avg_assembly_write_us = if self.assembly_write_count.load(Ordering::Relaxed) > 0 {
            self.assembly_write_ns.load(Ordering::Relaxed) as f64
                / self.assembly_write_count.load(Ordering::Relaxed) as f64
                / 1000.0
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

        let semaphore_pct = if avg_chunk_us > 0.0 {
            avg_semaphore_us / avg_chunk_us * 100.0
        } else {
            0.0
        };
        let download_pct = if avg_chunk_us > 0.0 {
            avg_download_us / avg_chunk_us * 100.0
        } else {
            0.0
        };
        let stream_pct = if avg_chunk_us > 0.0 {
            avg_stream_us / avg_chunk_us * 100.0
        } else {
            0.0
        };
        let verify_pct = if avg_chunk_us > 0.0 {
            avg_verify_us / avg_chunk_us * 100.0
        } else {
            0.0
        };

        log::info!(
            "[PROFILE #{count}] elapsed={elapsed:.1}s chunks={chunks} throughput={throughput_mbps:.1}MiB/s",
        );
        log::info!(
            "[PROFILE #{count}] chunk_avg={avg_chunk_us:.0}us semaphore={avg_semaphore_us:.0}us({semaphore_pct:.0}%) download={avg_download_us:.0}us({download_pct:.0}%) stream={avg_stream_us:.0}us({stream_pct:.0}%) verify={avg_verify_us:.0}us({verify_pct:.0}%)",
        );
        log::info!(
            "[PROFILE #{count}] assembly_avg={avg_assembly_total_us:.0}us decompress={avg_assembly_decompress_us:.0}us write={avg_assembly_write_us:.0}us",
        );

        let download_pct = (avg_download_us + avg_stream_us) / avg_chunk_us * 100.0;
        let overhead_pct = 100.0 - download_pct;
        log::info!(
            "[PROFILE #{count}] download_portion={download_pct:.1}% overhead_portion={overhead_pct:.1}%",
        );
    }
}

pub struct ChunkTimer<'a> {
    profiler: &'a PipelineProfiler,
    start: Instant,
    phase_start: Instant,
}

impl<'a> ChunkTimer<'a> {
    pub fn new(profiler: &'a PipelineProfiler) -> Self {
        let now = Instant::now();
        Self {
            profiler,
            start: now,
            phase_start: now,
        }
    }

    pub fn record_phase(&mut self, phase: ChunkPhase) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.phase_start);
        let ns = elapsed.as_nanos() as u64;

        match phase {
            ChunkPhase::SemaphoreWait => {
                self.profiler
                    .semaphore_wait_ns
                    .fetch_add(ns, Ordering::Relaxed);
                self.profiler
                    .semaphore_wait_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            ChunkPhase::Verify => {
                self.profiler.verify_ns.fetch_add(ns, Ordering::Relaxed);
                self.profiler.verify_count.fetch_add(1, Ordering::Relaxed);
            }
            ChunkPhase::DownloadWait => {
                self.profiler
                    .download_wait_ns
                    .fetch_add(ns, Ordering::Relaxed);
                self.profiler
                    .download_wait_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            ChunkPhase::StreamRead => {
                self.profiler
                    .stream_read_ns
                    .fetch_add(ns, Ordering::Relaxed);
                self.profiler
                    .stream_read_count
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        self.phase_start = now;
    }

    pub fn finish(self) {
        let elapsed = self.start.elapsed().as_nanos() as u64;
        self.profiler
            .chunk_total_ns
            .fetch_add(elapsed, Ordering::Relaxed);
        self.profiler
            .chunk_total_count
            .fetch_add(1, Ordering::Relaxed);
    }
}

pub enum ChunkPhase {
    SemaphoreWait,
    Verify,
    DownloadWait,
    StreamRead,
}

pub struct AssemblyTimer<'a> {
    profiler: &'a PipelineProfiler,
    start: Instant,
}

impl<'a> AssemblyTimer<'a> {
    pub fn new(profiler: &'a PipelineProfiler) -> Self {
        Self {
            profiler,
            start: Instant::now(),
        }
    }

    #[allow(dead_code)]
    pub fn record_decompress_time(&self, duration: Duration) {
        let ns = duration.as_nanos() as u64;
        self.profiler
            .assembly_decompress_ns
            .fetch_add(ns, Ordering::Relaxed);
        self.profiler
            .assembly_decompress_count
            .fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub fn record_write_time(&self, duration: Duration) {
        let ns = duration.as_nanos() as u64;
        self.profiler
            .assembly_write_ns
            .fetch_add(ns, Ordering::Relaxed);
        self.profiler
            .assembly_write_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn finish(self) {
        let elapsed = self.start.elapsed().as_nanos() as u64;
        self.profiler
            .assembly_total_ns
            .fetch_add(elapsed, Ordering::Relaxed);
        self.profiler
            .assembly_total_count
            .fetch_add(1, Ordering::Relaxed);
    }
}
