use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use reqwest::Client;

use crate::commands::sophon_downloader::api_scrape::{
    FrontDoorResponse, GameBranch, PackageBranch, SophonBuildData, SophonBuildResponse,
    SophonPatchBuildData, SophonPatchBuildResponse, SophonPatchManifestMeta,
};
use crate::commands::sophon_downloader::proto_parse::{
    SophonManifestProto, SophonPatchProto, decode_manifest, decode_patch_manifest,
};

use super::error::{SophonError, SophonResult};
use super::{FRONT_DOOR_URL, SOPHON_BUILD_URL_BASE};
use crate::commands::sophon_downloader::api_scrape::DownloadInfo;
use crate::commands::sophon_downloader::compute_content_manifest_hash;

pub struct ManifestWithHash {
    pub manifest: SophonManifestProto,
    pub hash: String,
}

pub async fn fetch_front_door(
    client: &Client,
    game_id: &str,
) -> SophonResult<(GameBranch, Option<PackageBranch>)> {
    let resp: FrontDoorResponse = client
        .get(FRONT_DOOR_URL)
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let branch = resp
        .data
        .game_branches
        .into_iter()
        .find(|b| b.game.id.eq_ignore_ascii_case(game_id))
        .ok_or_else(|| SophonError::UnknownGameId(game_id.into()))?;

    let pre = branch.pre_download.clone();
    Ok((branch, pre))
}

pub async fn fetch_manifest(
    client: &Client,
    dl: &DownloadInfo,
    manifest_id: &str,
) -> SophonResult<ManifestWithHash> {
    let url = dl.url_for(manifest_id);
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(120))
        .send()
        .await?
        .error_for_status()?;

    // Write to a temporary file before decompressing
    // This prevents holding both compressed and decompressed manifests in memory at
    // the same time (almost 100MB each)
    let raw = if dl.is_compressed() {
        let bytes = resp.bytes().await?;

        let tmp_path = tokio::task::spawn_blocking(move || {
            let tmp_path = manifest_temp_path();
            let mut f = std::fs::File::create(&tmp_path)?;
            f.write_all(&bytes)?;
            f.flush()?;
            Ok::<PathBuf, SophonError>(tmp_path)
        })
        .await??;

        tokio::task::spawn_blocking(move || {
            let raw = decompress_zstd_from_file(&tmp_path)?;
            let _ = std::fs::remove_file(&tmp_path);
            Ok::<Vec<u8>, SophonError>(raw)
        })
        .await??
    } else {
        resp.bytes().await?.to_vec()
    };

    let manifest: SophonManifestProto =
        decode_manifest(&raw).map_err(SophonError::ManifestDecode)?;
    let hash = compute_content_manifest_hash(&manifest);
    Ok(ManifestWithHash { manifest, hash })
}

/// Generates a unique temp file path for a manifest download.
/// Uses PID + timestamp to avoid collisions across concurrent calls.
fn manifest_temp_path() -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("sophon_manifest_{ts}_{}", std::process::id()))
}

/// Decompresses zstd data from a file, keeping only the decompressed output in
/// memory.
fn decompress_zstd_from_file(path: &PathBuf) -> SophonResult<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut decoder = zstd::Decoder::new(file)?;
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

pub async fn fetch_build(
    client: &Client,
    branch: &PackageBranch,
    tag: Option<&str>,
) -> SophonResult<SophonBuildData> {
    let tag_str = tag.unwrap_or(&branch.tag);
    let url = format!(
        "{}?branch={}&package_id={}&password={}&tag={}",
        SOPHON_BUILD_URL_BASE, branch.branch, branch.package_id, branch.password, tag_str,
    );

    let resp: SophonBuildResponse = client
        .get(&url)
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if resp.data.manifests.is_empty() {
        return Err(SophonError::NoManifests);
    }
    Ok(resp.data)
}

pub const SOPHON_PATCH_BUILD_URL_BASE: &str = concat!(
    "https://sg-public-api.hoyoverse.com",
    "/downloader/sophon_chunk/api/getPatchBuild"
);

pub async fn fetch_patch_build(
    client: &Client,
    branch: &PackageBranch,
) -> SophonResult<SophonPatchBuildData> {
    let url = format!(
        "{}?branch={}&package_id={}&password={}&tag={}",
        SOPHON_PATCH_BUILD_URL_BASE, branch.branch, branch.package_id, branch.password, branch.tag,
    );

    let resp: SophonPatchBuildResponse = client
        .post(&url)
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if resp.data.manifests.is_empty() {
        return Err(SophonError::NoManifests);
    }
    Ok(resp.data)
}

pub struct PatchManifestWithMeta {
    pub patch_manifest: SophonPatchProto,
    pub diff_download: DownloadInfo,
    pub matching_field: String,
}

pub async fn fetch_patch_manifest(
    client: &Client,
    meta: &SophonPatchManifestMeta,
) -> SophonResult<PatchManifestWithMeta> {
    let url = meta.manifest_download.url_for(&meta.manifest.id);
    let bytes = client
        .get(&url)
        .timeout(Duration::from_secs(120))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let raw = if meta.manifest_download.is_compressed() {
        tokio::task::spawn_blocking(move || zstd_decompress(&bytes)).await??
    } else {
        bytes.to_vec()
    };

    let patch_manifest =
        decode_patch_manifest(&raw).map_err(|e| SophonError::PatchManifestDecode(e.to_string()))?;

    Ok(PatchManifestWithMeta {
        patch_manifest,
        diff_download: meta.diff_download.clone(),
        matching_field: meta.matching_field.clone(),
    })
}

fn zstd_decompress(bytes: &[u8]) -> SophonResult<Vec<u8>> {
    use std::io::Read;
    let mut decoder = zstd::Decoder::new(bytes)?;
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

#[inline]
pub fn vo_lang_matches(matching_field: &str, vo_lang: &str) -> bool {
    let vo = vo_lang.as_bytes();
    let lower = matching_field.to_ascii_lowercase();
    if vo.eq_ignore_ascii_case(b"cn") {
        lower.contains("zh")
    } else if vo.eq_ignore_ascii_case(b"en") {
        lower.contains("en")
    } else if vo.eq_ignore_ascii_case(b"jp") {
        lower.contains("ja")
    } else if vo.eq_ignore_ascii_case(b"kr") {
        lower.contains("ko")
    } else {
        false
    }
}

#[inline]
pub fn is_known_vo_locale(matching_field: &str) -> bool {
    let f = matching_field.as_bytes();
    f.windows(5).any(|w| w.eq_ignore_ascii_case(b"en-us"))
        || f.windows(5).any(|w| w.eq_ignore_ascii_case(b"zh-cn"))
        || f.windows(5).any(|w| w.eq_ignore_ascii_case(b"zh-tw"))
        || f.windows(5).any(|w| w.eq_ignore_ascii_case(b"ko-kr"))
        || f.windows(5).any(|w| w.eq_ignore_ascii_case(b"ja-jp"))
}

#[inline]
pub fn parse_size(s: &str) -> SophonResult<u64> {
    s.parse()
        .map_err(|_| SophonError::InvalidSizeString(s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vo_lang_matches_cn() {
        assert!(vo_lang_matches("zh-cn", "cn"));
    }

    #[test]
    fn vo_lang_matches_en() {
        assert!(vo_lang_matches("en-us", "en"));
    }

    #[test]
    fn vo_lang_matches_jp() {
        assert!(vo_lang_matches("ja-jp", "jp"));
    }

    #[test]
    fn vo_lang_matches_kr() {
        assert!(vo_lang_matches("ko-kr", "kr"));
    }

    #[test]
    fn vo_lang_matches_wrong() {
        assert!(!vo_lang_matches("en-us", "jp"));
    }

    #[test]
    fn vo_lang_matches_game_field() {
        assert!(!vo_lang_matches("game", "en"));
    }

    #[test]
    fn is_known_vo_locale_all() {
        assert!(is_known_vo_locale("en-us"));
        assert!(is_known_vo_locale("zh-cn"));
        assert!(is_known_vo_locale("zh-tw"));
        assert!(is_known_vo_locale("ko-kr"));
        assert!(is_known_vo_locale("ja-jp"));
    }

    #[test]
    fn is_known_vo_locale_not_vo() {
        assert!(!is_known_vo_locale("game"));
        assert!(!is_known_vo_locale("cutscenes"));
    }

    #[test]
    fn parse_size_valid() {
        assert_eq!(parse_size("1024").unwrap(), 1024);
    }

    #[test]
    fn parse_size_invalid() {
        assert!(parse_size("abc").is_err());
    }

    #[test]
    fn parse_size_zero() {
        assert_eq!(parse_size("0").unwrap(), 0);
    }

    #[test]
    fn parse_size_negative_returns_error() {
        assert!(parse_size("-1").is_err());
    }

    #[test]
    fn parse_size_empty_returns_error() {
        assert!(parse_size("").is_err());
    }

    #[test]
    fn parse_size_whitespace_returns_error() {
        assert!(parse_size(" 1024 ").is_err());
    }

    #[test]
    fn parse_size_leading_zeros() {
        assert_eq!(parse_size("0001024").unwrap(), 1024);
    }

    #[test]
    fn parse_size_large_number() {
        assert_eq!(parse_size("999999999999").unwrap(), 999999999999);
    }

    #[test]
    fn vo_lang_matches_empty_both() {
        assert!(!vo_lang_matches("", ""));
    }

    #[test]
    fn vo_lang_matches_exact() {
        assert!(vo_lang_matches("en", "en"));
    }

    #[test]
    fn vo_lang_matches_case_insensitive_en() {
        assert!(vo_lang_matches("EN-US", "en"));
    }

    #[test]
    fn vo_lang_matches_case_insensitive_cn() {
        assert!(vo_lang_matches("ZH-CN", "cn"));
    }

    #[test]
    fn is_known_vo_locale_empty() {
        assert!(!is_known_vo_locale(""));
    }

    #[test]
    fn is_known_vo_locale_gibberish() {
        assert!(!is_known_vo_locale("xyz123"));
    }

    #[test]
    fn manifest_temp_path_returns_valid_path() {
        let path = manifest_temp_path();
        assert!(path.to_string_lossy().contains("sophon_manifest_"));
        assert!(path.parent().unwrap() == std::env::temp_dir());
    }

    #[test]
    fn decompress_zstd_from_file_roundtrip() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("data.zst");
        let original = b"hello zstd world!";
        let compressed = zstd::encode_all(std::io::Cursor::new(original), 3).unwrap();
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(&compressed).unwrap();
        drop(f);
        let decompressed = decompress_zstd_from_file(&file_path).unwrap();
        assert_eq!(decompressed, original);
    }

    #[test]
    fn decompress_zstd_from_file_missing_returns_error() {
        let path = std::path::PathBuf::from("/nonexistent/file.zst");
        let result = decompress_zstd_from_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn decompress_zstd_from_file_invalid_data_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("invalid.zst");
        std::fs::write(&file_path, b"not zstd compressed data").unwrap();
        let result = decompress_zstd_from_file(&file_path);
        assert!(result.is_err());
    }
}
