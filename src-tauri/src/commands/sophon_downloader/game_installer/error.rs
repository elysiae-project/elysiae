use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SophonError {
    #[error("HTTP request failed")]
    Http(
        #[from]
        #[source]
        reqwest::Error,
    ),

    #[error("IO error")]
    Io(
        #[from]
        #[source]
        std::io::Error,
    ),

    #[error("Task join error")]
    JoinError(
        #[from]
        #[source]
        tokio::task::JoinError,
    ),

    #[error("Semaphore error: {0}")]
    Semaphore(String),

    #[error("Failed to decode manifest")]
    ManifestDecode(
        #[from]
        #[source]
        prost::DecodeError,
    ),

    #[error("Failed to decompress data: {0}")]
    Decompression(String),

    #[error("Invalid asset name: {0}")]
    InvalidAssetName(String),

    #[error("Path traversal detected: {0}")]
    PathTraversal(PathBuf),

    #[error("MD5 mismatch for {item}: expected {expected}, got {actual}")]
    Md5Mismatch {
        item: String,
        expected: String,
        actual: String,
    },

    #[error("Size mismatch for {item}: expected {expected} bytes, got {actual} bytes")]
    SizeMismatch {
        item: String,
        expected: u64,
        actual: u64,
    },

    #[error("Unknown game ID: {0}")]
    UnknownGameId(String),

    #[error("API returned no manifests")]
    NoManifests,

    #[error("No game manifest found")]
    NoGameManifest,

    #[error("No voice manifest found for language: {0}")]
    NoVoiceManifest(String),

    #[error("No installed version found")]
    NoInstalledVersion,

    #[error("No preinstall available")]
    NoPreinstallAvailable,

    #[error("Preinstall marker not found for tag: {0}")]
    PreinstallMarkerNotFound(String),

    #[error("Download cancelled")]
    Cancelled,

    #[error("Failed to download chunk {chunk} after {attempts} attempts: {error}")]
    DownloadFailed {
        chunk: String,
        attempts: u32,
        error: String,
    },

    #[error("Failed to assemble file {file}: {error}")]
    AssemblyFailed { file: String, error: String },

    #[error("JSON error")]
    Json(
        #[from]
        #[source]
        serde_json::Error,
    ),

    #[error("Front-door branch index out of range")]
    BranchIndexOutOfRange,

    #[error("File index {index} out of bounds")]
    FileIndexOutOfBounds { index: usize },

    #[error("Temp dir index {index} out of bounds")]
    TmpDirIndexOutOfBounds { index: usize },
}

impl From<tokio::sync::AcquireError> for SophonError {
    fn from(err: tokio::sync::AcquireError) -> Self {
        SophonError::Semaphore(err.to_string())
    }
}

impl From<SophonError> for String {
    fn from(err: SophonError) -> Self {
        err.to_string()
    }
}

pub type SophonResult<T> = Result<T, SophonError>;
