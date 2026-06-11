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

    #[error("{kind} index {index} out of bounds")]
    IndexOutOfBounds { kind: &'static str, index: usize },

    #[error("API returned error (retcode={0}): {1}")]
    ApiError(i32, String),

    #[error("Plugin validation failed: {0}")]
    PluginValidationFailed(String),

    #[error("Failed to decode patch manifest: {0}")]
    PatchManifestDecode(String),

    #[error("HDiff patch failed for {file}: {error}")]
    HDiffPatchFailed { file: String, error: String },

    #[error("Original file missing for patch: {0}")]
    OriginalFileMissing(String),

    #[error("Patch chunk not found: {0}")]
    PatchChunkNotFound(String),

    #[error("Preinstall state file corrupted or missing: {0}")]
    PreinstallStateInvalid(String),

    #[error("No space available at {path}: need {needed}, have {available}")]
    NoSpaceAvailable {
        path: String,
        needed: u64,
        available: u64,
    },

    #[error("Resume failed: {message}")]
    ResumeFailed { message: String },

    #[error("Invalid size string: {0}")]
    InvalidSizeString(String),
}

/// Converts SophonError to a plain string for the Tauri IPC boundary.
/// Structured error handling should match on SophonError before calling this.
impl From<SophonError> for String {
    fn from(err: SophonError) -> Self {
        err.to_string()
    }
}

pub type SophonResult<T> = Result<T, SophonError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_md5_mismatch() {
        let err = SophonError::Md5Mismatch {
            item: "file.pkg".to_string(),
            expected: "abc123".to_string(),
            actual: "def456".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("file.pkg"));
        assert!(msg.contains("abc123"));
        assert!(msg.contains("def456"));
    }

    #[test]
    fn error_display_size_mismatch() {
        let err = SophonError::SizeMismatch {
            item: "data.bin".to_string(),
            expected: 1024,
            actual: 512,
        };
        let msg = err.to_string();
        assert!(msg.contains("data.bin"));
        assert!(msg.contains("1024"));
        assert!(msg.contains("512"));
    }

    #[test]
    fn error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let sophon_err: SophonError = io_err.into();
        assert!(matches!(sophon_err, SophonError::Io(_)));
    }

    #[test]
    fn error_display_cancelled() {
        let err = SophonError::Cancelled;
        assert_eq!(err.to_string(), "Download cancelled");
    }

    #[test]
    fn error_display_path_traversal() {
        let err = SophonError::PathTraversal(PathBuf::from("../../etc/passwd"));
        let msg = err.to_string();
        assert!(msg.contains("../../etc/passwd"));
    }

    #[test]
    fn error_from_semaphore_acquire() {
        let sophon_err = SophonError::Semaphore("no permits available".to_string());
        assert!(matches!(sophon_err, SophonError::Semaphore(_)));
    }

    #[test]
    fn error_into_string() {
        let s: String = SophonError::Cancelled.into();
        assert_eq!(s, "Download cancelled");
        let s: String = SophonError::PathTraversal(PathBuf::from("/bad/path")).into();
        assert!(s.contains("/bad/path"));
        let s: String = SophonError::NoManifests.into();
        assert!(!s.is_empty());
    }
}
