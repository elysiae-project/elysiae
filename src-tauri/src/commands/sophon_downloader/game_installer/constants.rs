pub const MAX_RETRIES: u32 = 4;
pub const ASSEMBLY_CONCURRENCY: usize = 4;
pub const ASSEMBLY_CHANNEL_SIZE: usize = ASSEMBLY_CONCURRENCY * 4;
pub const VERSION_FILE_NAME: &str = ".sophon_version";
pub const VERIFICATION_CACHE_FILE: &str = ".sophon_verify_cache";

pub const DOWNLOAD_STREAM_BUFFER_SIZE: usize = 256 * 1024;
pub const FILE_WRITE_BUFFER_SIZE: usize = 1024 * 1024;
pub const DECOMPRESSION_BUFFER_SIZE: usize = 1024 * 1024;
pub const MD5_HASH_BUFFER_SIZE: usize = 1024 * 1024;

pub const PROGRESS_UPDATE_INTERVAL_MS: u64 = 1000;

pub const ADAPTIVE_MIN_CONCURRENCY: usize = 4;
pub const ADAPTIVE_MAX_CONCURRENCY: usize = 32;
pub const ADAPTIVE_INITIAL_CONCURRENCY: usize = 8;
pub const ADAPTIVE_WINDOW_SECS: u64 = 2;

pub const FRONT_DOOR_URL: &str = concat!(
    "https://sg-hyp-api.hoyoverse.com",
    "/hyp/hyp-connect/api/getGameBranches?&launcher_id=VYTpXlbWo8"
);
pub const SOPHON_BUILD_URL_BASE: &str = concat!(
    "https://sg-public-api.hoyoverse.com",
    "/downloader/sophon_chunk/api/getBuild"
);
