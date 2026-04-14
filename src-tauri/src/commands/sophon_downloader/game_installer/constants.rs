//! Configuration constants for the Sophon downloader.

/// Maximum retry attempts for failed chunk downloads.
pub const MAX_RETRIES: u32 = 4;
/// Maximum concurrent file assembly tasks.
pub const ASSEMBLY_CONCURRENCY: usize = 4;
/// Size of the channel buffer for assembly task scheduling.
pub const ASSEMBLY_CHANNEL_SIZE: usize = ASSEMBLY_CONCURRENCY * 4;
/// Filename for the installed version marker file.
pub const VERSION_FILE_NAME: &str = ".sophon_version";
/// Filename for the MD5 verification cache.
pub const VERIFICATION_CACHE_FILE: &str = ".sophon_verify_cache";

/// Buffer size for download stream writes (256 KiB).
pub const DOWNLOAD_STREAM_BUFFER_SIZE: usize = 256 * 1024;
/// Buffer size for file writes during assembly (1 MiB).
pub const FILE_WRITE_BUFFER_SIZE: usize = 1024 * 1024;
/// Buffer size for zstd decompression (1 MiB).
pub const DECOMPRESSION_BUFFER_SIZE: usize = 1024 * 1024;
/// Buffer size for MD5 hashing (1 MiB).
pub const MD5_HASH_BUFFER_SIZE: usize = 1024 * 1024;

/// Minimum interval between progress updates (ms).
pub const PROGRESS_UPDATE_INTERVAL_MS: u64 = 1000;

/// Minimum concurrent downloads in adaptive mode.
pub const ADAPTIVE_MIN_CONCURRENCY: usize = 4;
/// Maximum concurrent downloads in adaptive mode.
pub const ADAPTIVE_MAX_CONCURRENCY: usize = 32;
/// Initial concurrent downloads in adaptive mode.
pub const ADAPTIVE_INITIAL_CONCURRENCY: usize = 8;
/// Time window for throughput measurement (seconds).
pub const ADAPTIVE_WINDOW_SECS: u64 = 2;

/// HoYoverse front-door API endpoint for game branches.
pub const FRONT_DOOR_URL: &str = concat!(
    "https://sg-hyp-api.hoyoverse.com",
    "/hyp/hyp-connect/api/getGameBranches?&launcher_id=VYTpXlbWo8"
);
/// Base URL for Sophon build manifest API.
pub const SOPHON_BUILD_URL_BASE: &str = concat!(
    "https://sg-public-api.hoyoverse.com",
    "/downloader/sophon_chunk/api/getBuild"
);
