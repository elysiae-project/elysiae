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

impl SophonError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http(_)
            | Self::Io(_)
            | Self::Md5Mismatch { .. }
            | Self::SizeMismatch { .. }
            | Self::ResumeFailed { .. }
            | Self::DownloadFailed { .. }
            | Self::Decompression(_) => true,
            Self::Cancelled
            | Self::NoManifests
            | Self::NoGameManifest
            | Self::NoVoiceManifest(_)
            | Self::NoInstalledVersion
            | Self::NoPreinstallAvailable
            | Self::PathTraversal(_)
            | Self::InvalidAssetName(_)
            | Self::NoSpaceAvailable { .. }
            | Self::PatchManifestDecode(_)
            | Self::PluginValidationFailed(_)
            | Self::HDiffPatchFailed { .. }
            | Self::OriginalFileMissing(_)
            | Self::PreinstallStateInvalid(_)
            | Self::PatchChunkNotFound(_)
            | Self::ApiError(_, _)
            | Self::UnknownGameId(_)
            | Self::JoinError(_)
            | Self::Semaphore(_)
            | Self::ManifestDecode(_)
            | Self::AssemblyFailed { .. }
            | Self::IndexOutOfBounds { .. }
            | Self::InvalidSizeString(_) => false,
        }
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
    fn error_to_string() {
        let s = SophonError::Cancelled.to_string();
        assert_eq!(s, "Download cancelled");
        let s = SophonError::PathTraversal(PathBuf::from("/bad/path")).to_string();
        assert!(s.contains("/bad/path"));
        let s = SophonError::NoManifests.to_string();
        assert!(!s.is_empty());
    }

    #[tokio::test]
    async fn error_display_http() {
        let result = reqwest::Client::new()
            .get("https://192.0.2.1:1/nonexistent")
            .timeout(std::time::Duration::from_millis(1))
            .send()
            .await;
        let reqwest_err = result.unwrap_err();
        let err = SophonError::Http(reqwest_err);
        let msg = err.to_string();
        assert!(!msg.is_empty());
    }

    #[test]
    fn error_display_join_error() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.spawn(async { panic!("intentional panic") });
        let result = rt.block_on(handle);
        let err = SophonError::JoinError(result.unwrap_err());
        let msg = err.to_string();
        assert!(!msg.is_empty());
    }

    #[test]
    fn error_display_manifest_decode() {
        let decode_err = prost::DecodeError::new("invalid wire type");
        let err = SophonError::ManifestDecode(decode_err);
        let msg = err.to_string();
        assert!(msg.contains("manifest"), "msg={msg}");
    }

    #[test]
    fn error_display_decompression() {
        let err = SophonError::Decompression("zstd error".to_string());
        assert_eq!(err.to_string(), "Failed to decompress data: zstd error");
    }

    #[test]
    fn error_display_invalid_asset_name() {
        let err = SophonError::InvalidAssetName("file\0.txt".to_string());
        let msg = err.to_string();
        assert!(msg.contains("file"), "msg={msg}");
    }

    #[test]
    fn error_display_unknown_game_id() {
        let err = SophonError::UnknownGameId("xyz".to_string());
        assert_eq!(err.to_string(), "Unknown game ID: xyz");
    }

    #[test]
    fn error_display_no_manifests() {
        assert_eq!(
            SophonError::NoManifests.to_string(),
            "API returned no manifests"
        );
    }

    #[test]
    fn error_display_no_game_manifest() {
        assert_eq!(
            SophonError::NoGameManifest.to_string(),
            "No game manifest found"
        );
    }

    #[test]
    fn error_display_no_voice_manifest() {
        let err = SophonError::NoVoiceManifest("ja-jp".to_string());
        assert_eq!(
            err.to_string(),
            "No voice manifest found for language: ja-jp"
        );
    }

    #[test]
    fn error_display_no_installed_version() {
        assert_eq!(
            SophonError::NoInstalledVersion.to_string(),
            "No installed version found"
        );
    }

    #[test]
    fn error_display_no_preinstall_available() {
        assert_eq!(
            SophonError::NoPreinstallAvailable.to_string(),
            "No preinstall available"
        );
    }

    #[test]
    fn error_display_download_failed() {
        let err = SophonError::DownloadFailed {
            chunk: "chunk_001".to_string(),
            attempts: 3,
            error: "timeout".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("chunk_001"));
        assert!(msg.contains("3"));
        assert!(msg.contains("timeout"));
    }

    #[test]
    fn error_display_assembly_failed() {
        let err = SophonError::AssemblyFailed {
            file: "data.pak".to_string(),
            error: "md5 mismatch".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("data.pak"));
        assert!(msg.contains("md5 mismatch"));
    }

    #[test]
    fn error_display_index_out_of_bounds() {
        let err = SophonError::IndexOutOfBounds {
            kind: "file_idx",
            index: 42,
        };
        let msg = err.to_string();
        assert!(msg.contains("file_idx"));
        assert!(msg.contains("42"));
    }

    #[test]
    fn error_display_plugin_validation_failed() {
        let err = SophonError::PluginValidationFailed("checksum mismatch".to_string());
        let msg = err.to_string();
        assert!(msg.contains("checksum mismatch"));
    }

    #[test]
    fn error_display_patch_manifest_decode() {
        let err = SophonError::PatchManifestDecode("truncated data".to_string());
        let msg = err.to_string();
        assert!(msg.contains("truncated data"));
    }

    #[test]
    fn error_display_hdiff_patch_failed() {
        let err = SophonError::HDiffPatchFailed {
            file: "data.bin".to_string(),
            error: "corrupt diff".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("data.bin"));
        assert!(msg.contains("corrupt diff"));
    }

    #[test]
    fn error_display_original_file_missing() {
        let err = SophonError::OriginalFileMissing("old/data.bin".to_string());
        let msg = err.to_string();
        assert!(msg.contains("old/data.bin"));
    }

    #[test]
    fn error_display_patch_chunk_not_found() {
        let err = SophonError::PatchChunkNotFound("patch_001.hdiff".to_string());
        let msg = err.to_string();
        assert!(msg.contains("patch_001.hdiff"));
    }

    #[test]
    fn error_display_preinstall_state_invalid() {
        let err = SophonError::PreinstallStateInvalid("corrupted json".to_string());
        let msg = err.to_string();
        assert!(msg.contains("corrupted json"));
    }

    #[test]
    fn error_display_no_space_available() {
        let err = SophonError::NoSpaceAvailable {
            path: "/game".to_string(),
            needed: 1_000_000_000,
            available: 500_000_000,
        };
        let msg = err.to_string();
        assert!(msg.contains("/game"));
        assert!(msg.contains("1000000000"));
        assert!(msg.contains("500000000"));
    }

    #[test]
    fn error_display_resume_failed() {
        let err = SophonError::ResumeFailed {
            message: "file size changed".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("file size changed"));
    }

    #[test]
    fn error_display_invalid_size_string() {
        let err = SophonError::InvalidSizeString("abc".to_string());
        let msg = err.to_string();
        assert!(msg.contains("abc"));
    }

    #[test]
    fn error_from_join_error() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.spawn(async { panic!("fail") });
        let join_err = rt.block_on(handle).unwrap_err();
        let sophon_err: SophonError = join_err.into();
        assert!(matches!(sophon_err, SophonError::JoinError(_)));
    }

    #[test]
    fn error_from_manifest_decode() {
        let decode_err = prost::DecodeError::new("test");
        let sophon_err: SophonError = decode_err.into();
        assert!(matches!(sophon_err, SophonError::ManifestDecode(_)));
    }

    #[test]
    fn error_impl_std_error() {
        use std::error::Error;
        let err = SophonError::NoManifests;
        assert!(err.source().is_none());
        let err = SophonError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "test"));
        assert!(err.source().is_some());
    }
}
