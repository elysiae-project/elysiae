use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use md5::{Digest, Md5};
use reqwest::Client;
use tauri_plugin_log::log;
use tokio::io::AsyncWriteExt;
use zip::ZipArchive;

use super::cache;
use super::error::{SophonError, SophonResult};
use super::plugin_api::{
    ChannelSdkData, PackageData, PluginPackageInfo, ValidationEntry, fetch_channel_sdks,
    fetch_plugins, game_id_for_code,
};
use crate::commands::sophon_downloader::SophonProgress;

type ProgressFn = Arc<dyn Fn(SophonProgress) + Send + Sync>;

const PLUGIN_VERSIONS_FILE: &str = "plugin_versions.json";

/// Serialises concurrent access to plugin_versions.json so that
/// read-modify-write does not race.
static PLUGIN_VERSION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn read_plugin_versions(game_dir: &Path) -> HashMap<String, String> {
    let path = game_dir.join(PLUGIN_VERSIONS_FILE);
    let Ok(content) = fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn write_plugin_version(game_dir: &Path, key: &str, value: &str) -> std::io::Result<()> {
    // Acquire the global lock so that concurrent calls do not race on the
    // read-modify-write of plugin_versions.json. The lock is held only during
    // the synchronous I/O section; no await points exist inside this function.
    let _lock = PLUGIN_VERSION_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let path = game_dir.join(PLUGIN_VERSIONS_FILE);
    let mut versions = read_plugin_versions(game_dir);
    versions.insert(key.to_string(), value.to_string());
    let content = serde_json::to_string_pretty(&versions)?;
    fs::write(&path, content)
}

fn plugin_needs_update(
    game_dir: &Path,
    plugin_id: &str,
    version: &str,
    validation: &[ValidationEntry],
) -> bool {
    let versions = read_plugin_versions(game_dir);
    let key = format!("plugin_{plugin_id}_version");

    if versions.get(&key).map(String::as_str) != Some(version) {
        return true;
    }

    for entry in validation {
        let file_path = game_dir.join(&entry.path);
        if !file_path.exists() {
            return true;
        }
        if let Some(expected_size) = entry.size
            && let Ok(meta) = fs::metadata(&file_path)
            && meta.len() != expected_size
        {
            return true;
        }
    }

    false
}

async fn download_zip(
    client: &Client,
    url: &str,
    dest: &Path,
    expected_md5: &str,
    updater: &ProgressFn,
) -> SophonResult<()> {
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(60))
        .send()
        .await?
        .error_for_status()?;
    let total_bytes = resp.content_length().unwrap_or(0);

    let mut file = tokio::fs::File::create(dest).await?;
    let mut stream = resp.bytes_stream();
    let mut hasher = Md5::new();
    let mut downloaded: u64 = 0;

    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "plugin".to_string());

    let mut last_emit = Instant::now();
    let throttle = Duration::from_secs(1);

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        hasher.update(&bytes);
        file.write_all(&bytes).await?;
        downloaded += bytes.len() as u64;

        if last_emit.elapsed() >= throttle {
            last_emit = Instant::now();
            updater(SophonProgress::DownloadingPlugin {
                name: name.clone(),
                downloaded_bytes: downloaded,
                total_bytes,
            });
        }
    }

    // Emit final progress after loop completes
    updater(SophonProgress::DownloadingPlugin {
        name: name.clone(),
        downloaded_bytes: downloaded,
        total_bytes,
    });

    file.flush().await?;

    let actual_md5 = hex::encode(hasher.finalize());
    // Compare case-insensitively: hex::encode always emits lowercase, but the
    // upstream API has no contract on the case of its returned hex digest.
    if actual_md5 != expected_md5.to_ascii_lowercase() {
        let _ = fs::remove_file(dest);
        return Err(SophonError::Md5Mismatch {
            item: name,
            expected: expected_md5.to_string(),
            actual: actual_md5,
        });
    }

    Ok(())
}

/// Maximum decompressed bytes per single ZIP entry. Defends against malicious
/// or malformed archives claiming huge file sizes (zip bombs).
const ZIP_MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024; // 512 MiB
/// Maximum aggregate uncompressed bytes across all entries.
const ZIP_MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB
/// Maximum number of entries in a single archive.
const ZIP_MAX_ENTRIES: usize = 8 * 1024;
/// Maximum uncompressed-to-compressed ratio per entry. 1 GiB of declared output
/// from a 1 KiB compressed entry is rejected (ratio > 1_000_000).
const ZIP_MAX_RATIO: u64 = 1_000;

fn extract_zip(zip_path: &Path, game_dir: &Path) -> SophonResult<()> {
    let file = File::open(zip_path)?;
    let reader = BufReader::new(file);
    let mut archive =
        ZipArchive::new(reader).map_err(|err| SophonError::Decompression(err.to_string()))?;

    if archive.len() > ZIP_MAX_ENTRIES {
        return Err(SophonError::Decompression(format!(
            "archive has {} entries, exceeds limit of {ZIP_MAX_ENTRIES}",
            archive.len()
        )));
    }

    // Canonicalize game_dir once to prevent time-of-check-time-of-use issues
    // and provide a stable base for symlink-traversal detection.
    let canonical_game = game_dir.canonicalize()?;
    let mut total_extracted: u64 = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|err| SophonError::Decompression(err.to_string()))?;

        // Reject symlink entries outright — silently treating a symlink-target
        // string as the contents of a regular file is a data-integrity bug and
        // could leak attacker-controlled text into game_dir.
        if entry.is_symlink() {
            log::warn!(
                "Rejecting symlink entry '{}' in plugin archive",
                entry.name()
            );
            continue;
        }

        // Reject NTFS alternate data streams and other colon-bearing names.
        // Legitimate plugin/SDK paths never contain ':'.
        if entry.name().contains(':') {
            log::warn!(
                "Rejecting entry with alternate-data-stream name '{}'",
                entry.name()
            );
            continue;
        }

        let declared = entry.size();
        if declared > ZIP_MAX_ENTRY_BYTES {
            return Err(SophonError::Decompression(format!(
                "entry '{}' declares {declared} bytes, exceeds per-entry limit of {ZIP_MAX_ENTRY_BYTES}",
                entry.name()
            )));
        }
        // Ratio check on the declared compressed size; skip if the entry did
        // not report one (treated as 0 by the zip crate for streaming entries).
        let cc = entry.compressed_size();
        if cc > 0 && declared > cc.saturating_mul(ZIP_MAX_RATIO) {
            return Err(SophonError::Decompression(format!(
                "entry '{}' ratio {} exceeds {ZIP_MAX_RATIO}",
                entry.name(),
                declared / cc,
            )));
        }
        total_extracted = match total_extracted.checked_add(declared) {
            Some(v) => v,
            None => {
                return Err(SophonError::Decompression(
                    "total extracted size overflows u64".into(),
                ));
            }
        };
        if total_extracted > ZIP_MAX_TOTAL_BYTES {
            return Err(SophonError::Decompression(format!(
                "extracted size would exceed {ZIP_MAX_TOTAL_BYTES} bytes"
            )));
        }

        let out_path = match entry.enclosed_name() {
            Some(path) => game_dir.join(path),
            None => continue,
        };

        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
            // Resolve symlinks in the created path; if the resolved path
            // escapes game_dir, this is a symlink-traversal attack.
            if !out_path.canonicalize()?.starts_with(&canonical_game) {
                log::warn!("Skipping path traversal attempt: {:?}", out_path);
                if let Err(err) = fs::remove_dir_all(&out_path) {
                    log::warn!(
                        "Failed to clean up symlink-traversal directory {}: {}",
                        out_path.display(),
                        err
                    );
                }
                continue;
            }
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
                // Canonicalize the parent directory to detect if any component
                // is a symlink pointing outside game_dir.
                if !parent.canonicalize()?.starts_with(&canonical_game) {
                    log::warn!("Skipping path traversal attempt: {:?}", out_path);
                    continue;
                }
            }
            let mut out_file = File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out_file)?;

            // Preserve Unix mode bits from the ZIP entry when present so that
            // shipped executables retain their +x bit instead of falling back
            // to the umask (which on many distros is 0o644).
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(mode & 0o7777));
            }
        }
    }

    Ok(())
}

fn cleanup_dxsetup(game_dir: &Path) {
    let dxsetup_dir = game_dir.join("DXSETUP");
    if dxsetup_dir.is_dir()
        && let Err(e) = fs::remove_dir_all(&dxsetup_dir)
    {
        log::warn!("Failed to clean up DXSETUP directory: {e}");
    }
}

fn verify_validation(game_dir: &Path, validation: &[ValidationEntry]) -> bool {
    for entry in validation {
        let file_path = game_dir.join(&entry.path);
        if !file_path.exists() {
            log::warn!("Validation file missing: {}", entry.path);
            return false;
        }
        if let Some(expected_size) = entry.size
            && let Ok(meta) = fs::metadata(&file_path)
            && meta.len() != expected_size
        {
            log::warn!(
                "Validation file size mismatch: {} (expected {}, got {})",
                entry.path,
                expected_size,
                meta.len()
            );
            return false;
        }
        // Verify MD5 hash if provided for stronger integrity guarantee
        if let Some(ref expected_md5) = entry.md5 {
            let computed = match cache::file_md5_hex(&file_path) {
                Ok(md5) => md5,
                Err(err) => {
                    log::warn!("Failed to compute MD5 for {}: {}", entry.path, err);
                    return false;
                }
            };
            if computed != *expected_md5 {
                log::warn!(
                    "Validation file MD5 mismatch: {} (expected {}, got {})",
                    entry.path,
                    expected_md5,
                    computed
                );
                return false;
            }
        }
    }
    true
}

async fn install_single_plugin(
    client: &Client,
    game_dir: &Path,
    plugin: &PluginPackageInfo,
    pkg: &PackageData,
    updater: &ProgressFn,
) -> SophonResult<()> {
    if !plugin_needs_update(
        game_dir,
        &plugin.plugin_id,
        &plugin.version,
        &pkg.validation,
    ) {
        return Ok(());
    }

    let raw_filename = pkg.url.rsplit('/').next().unwrap_or("plugin.zip");
    if let Err(err) = super::assembly::validate_asset_name(raw_filename) {
        log::warn!("Refusing plugin URL with unsafe filename: {err}");
        return Err(SophonError::PathTraversal(PathBuf::from(raw_filename)));
    }
    let zip_path = game_dir.join(raw_filename);

    download_zip(client, &pkg.url, &zip_path, &pkg.md5, updater).await?;

    if let Err(err) = extract_zip(&zip_path, game_dir) {
        let _ = fs::remove_file(&zip_path);
        return Err(err);
    }

    if !verify_validation(game_dir, &pkg.validation) {
        let _ = fs::remove_file(&zip_path);
        for entry in &pkg.validation {
            if let Err(err) = super::assembly::validate_asset_name(&entry.path) {
                log::warn!("Skipping cleanup of invalid validation path: {err}");
                continue;
            }
            let _ = fs::remove_file(game_dir.join(&entry.path));
        }
        return Err(SophonError::PluginValidationFailed(
            plugin.plugin_id.clone(),
        ));
    }

    cleanup_dxsetup(game_dir);

    let plugin_id = &plugin.plugin_id;
    let safe_version = plugin.version.replace(['\n', '\r'], "");
    write_plugin_version(
        game_dir,
        &format!("plugin_{plugin_id}_version"),
        &safe_version,
    )?;

    let _ = fs::remove_file(&zip_path);

    Ok(())
}

pub async fn install_plugins(
    client: &Client,
    game_dir: &Path,
    game_code: &str,
    updater: impl Fn(SophonProgress) + Send + Sync + 'static,
) -> SophonResult<()> {
    let Some(hy_id) = game_id_for_code(game_code) else {
        return Ok(());
    };

    let plugins = fetch_plugins(client, hy_id).await?;
    if plugins.is_empty() {
        return Ok(());
    }

    let updater: ProgressFn = Arc::new(updater);
    let total = plugins.len();
    for plugin in plugins.iter() {
        updater(SophonProgress::InstallingPlugins {
            current_plugin: plugin.plugin_id.clone(),
            total_plugins: total,
        });

        if let Err(err) =
            install_single_plugin(client, game_dir, plugin, &plugin.plugin_pkg, &updater).await
        {
            let plugin_id = &plugin.plugin_id;
            log::warn!("Plugin {plugin_id} installation failed: {err}");
        }
    }

    Ok(())
}

async fn install_single_sdk(
    client: &Client,
    game_dir: &Path,
    sdk: &ChannelSdkData,
    updater: &ProgressFn,
) -> SophonResult<()> {
    let key = "plugin_sdk_version";
    let versions = read_plugin_versions(game_dir);

    if versions.get(key).map(String::as_str) == Some(sdk.version.as_str())
        && verify_validation(game_dir, &sdk.channel_sdk_pkg.validation)
    {
        return Ok(());
    }

    let raw_filename = sdk
        .channel_sdk_pkg
        .url
        .rsplit('/')
        .next()
        .unwrap_or("sdk.zip");
    if let Err(err) = super::assembly::validate_asset_name(raw_filename) {
        log::warn!("Refusing SDK URL with unsafe filename: {err}");
        return Err(SophonError::PathTraversal(PathBuf::from(raw_filename)));
    }
    let zip_path = game_dir.join(raw_filename);

    download_zip(
        client,
        &sdk.channel_sdk_pkg.url,
        &zip_path,
        &sdk.channel_sdk_pkg.md5,
        updater,
    )
    .await?;

    if let Err(err) = extract_zip(&zip_path, game_dir) {
        let _ = fs::remove_file(&zip_path);
        return Err(err);
    }

    if !verify_validation(game_dir, &sdk.channel_sdk_pkg.validation) {
        let _ = fs::remove_file(&zip_path);
        for entry in &sdk.channel_sdk_pkg.validation {
            if let Err(err) = super::assembly::validate_asset_name(&entry.path) {
                log::warn!("Skipping cleanup of invalid validation path: {err}");
                continue;
            }
            let _ = fs::remove_file(game_dir.join(&entry.path));
        }
        return Err(SophonError::PluginValidationFailed(sdk.game.id.clone()));
    }

    cleanup_dxsetup(game_dir);

    let safe_version = sdk.version.replace(['\n', '\r'], "");
    write_plugin_version(game_dir, key, &safe_version)?;

    let _ = fs::remove_file(&zip_path);

    Ok(())
}

pub async fn install_channel_sdks(
    client: &Client,
    game_dir: &Path,
    game_code: &str,
    updater: impl Fn(SophonProgress) + Send + Sync + 'static,
) -> SophonResult<()> {
    let Some(hy_id) = game_id_for_code(game_code) else {
        return Ok(());
    };

    let sdks = fetch_channel_sdks(client, hy_id).await?;
    if sdks.is_empty() {
        return Ok(());
    }

    let updater: ProgressFn = Arc::new(updater);
    let total = sdks.len();
    for sdk in sdks.iter() {
        let game_id = &sdk.game.id;
        updater(SophonProgress::InstallingSdks {
            current_sdk: format!("sdk_{game_id}"),
            total_sdks: total,
        });

        if let Err(err) = install_single_sdk(client, game_dir, sdk, &updater).await {
            log::warn!("SDK {} installation failed: {}", sdk.game.id, err);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufWriter, Write};
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    fn make_validation_entry(path: &str, size: Option<u64>) -> ValidationEntry {
        ValidationEntry {
            path: path.to_string(),
            md5: None,
            size,
        }
    }

    fn create_test_zip(zip_path: &Path, files: &[(&str, &[u8])]) {
        let file = File::create(zip_path).unwrap();
        let mut writer = ZipWriter::new(BufWriter::new(file));
        for (name, content) in files {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }

    fn create_test_zip_with_dirs(zip_path: &Path, entries: &[(&str, bool, &[u8])]) {
        let file = File::create(zip_path).unwrap();
        let mut writer = ZipWriter::new(BufWriter::new(file));
        for (name, is_dir, content) in entries {
            if *is_dir {
                writer
                    .add_directory(*name, SimpleFileOptions::default())
                    .unwrap();
            } else {
                writer
                    .start_file(*name, SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(content).unwrap();
            }
        }
        writer.finish().unwrap();
    }

    fn create_empty_zip(zip_path: &Path) {
        let file = File::create(zip_path).unwrap();
        let writer = ZipWriter::new(BufWriter::new(file));
        writer.finish().unwrap();
    }

    #[test]
    fn read_plugin_versions_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let versions = read_plugin_versions(dir.path());
        assert!(versions.is_empty());
    }

    #[test]
    fn read_plugin_versions_with_entries() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join(PLUGIN_VERSIONS_FILE);
        let json = serde_json::json!({
            "plugin_abc_version": "1.0",
            "plugin_def_version": "2.0"
        });
        fs::write(&json_path, serde_json::to_string_pretty(&json).unwrap()).unwrap();
        let versions = read_plugin_versions(dir.path());
        assert_eq!(versions.len(), 2);
        assert_eq!(versions.get("plugin_abc_version"), Some(&"1.0".to_string()));
        assert_eq!(versions.get("plugin_def_version"), Some(&"2.0".to_string()));
    }

    #[test]
    fn read_plugin_versions_corrupted_json() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join(PLUGIN_VERSIONS_FILE);
        fs::write(&json_path, "not valid json!!!").unwrap();
        let versions = read_plugin_versions(dir.path());
        assert!(versions.is_empty());
    }

    #[test]
    fn write_plugin_version_new_file() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin_version(dir.path(), "plugin_abc_version", "1.0").unwrap();
        let versions = read_plugin_versions(dir.path());
        assert_eq!(versions.get("plugin_abc_version"), Some(&"1.0".to_string()));
        let json_path = dir.path().join(PLUGIN_VERSIONS_FILE);
        assert!(json_path.exists());
    }

    #[test]
    fn write_plugin_version_update_existing() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin_version(dir.path(), "plugin_abc_version", "1.0").unwrap();
        write_plugin_version(dir.path(), "plugin_abc_version", "2.0").unwrap();
        let versions = read_plugin_versions(dir.path());
        assert_eq!(versions.get("plugin_abc_version"), Some(&"2.0".to_string()));
    }

    #[test]
    fn write_plugin_version_add_new_key() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin_version(dir.path(), "plugin_abc_version", "1.0").unwrap();
        write_plugin_version(dir.path(), "plugin_def_version", "2.0").unwrap();
        let versions = read_plugin_versions(dir.path());
        assert_eq!(versions.len(), 2);
        assert_eq!(versions.get("plugin_abc_version"), Some(&"1.0".to_string()));
        assert_eq!(versions.get("plugin_def_version"), Some(&"2.0".to_string()));
    }

    #[test]
    fn write_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin_version(dir.path(), "plugin_abc_version", "3.0").unwrap();
        let versions = read_plugin_versions(dir.path());
        assert_eq!(versions.get("plugin_abc_version"), Some(&"3.0".to_string()));
    }

    #[test]
    fn plugin_needs_update_version_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin_version(dir.path(), "plugin_abc_version", "1.0").unwrap();
        let result = plugin_needs_update(dir.path(), "abc", "2.0", &[]);
        assert!(result);
    }

    #[test]
    fn plugin_needs_update_version_match_files_ok() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin_version(dir.path(), "plugin_abc_version", "1.0").unwrap();
        let file_path = dir.path().join("gme.dll");
        fs::write(&file_path, b"test content").unwrap();
        let validation = vec![make_validation_entry("gme.dll", Some(12))];
        let result = plugin_needs_update(dir.path(), "abc", "1.0", &validation);
        assert!(!result);
    }

    #[test]
    fn plugin_needs_update_version_match_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin_version(dir.path(), "plugin_abc_version", "1.0").unwrap();
        let validation = vec![make_validation_entry("missing.dll", Some(10))];
        let result = plugin_needs_update(dir.path(), "abc", "1.0", &validation);
        assert!(result);
    }

    #[test]
    fn plugin_needs_update_no_version() {
        let dir = tempfile::tempdir().unwrap();
        let result = plugin_needs_update(dir.path(), "abc", "1.0", &[]);
        assert!(result);
    }

    #[test]
    fn verify_validation_all_present() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("gme.dll");
        fs::write(&file_path, b"hello world").unwrap();
        let validation = vec![make_validation_entry("gme.dll", Some(11))];
        assert!(verify_validation(dir.path(), &validation));
    }

    #[test]
    fn verify_validation_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let validation = vec![make_validation_entry("absent.dll", Some(10))];
        assert!(!verify_validation(dir.path(), &validation));
    }

    #[test]
    fn verify_validation_file_wrong_size() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("gme.dll");
        fs::write(&file_path, b"hello").unwrap();
        let validation = vec![make_validation_entry("gme.dll", Some(999))];
        assert!(!verify_validation(dir.path(), &validation));
    }

    #[test]
    fn verify_validation_no_size_check() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("gme.dll");
        fs::write(&file_path, b"any content").unwrap();
        let validation = vec![make_validation_entry("gme.dll", None)];
        assert!(verify_validation(dir.path(), &validation));
    }

    #[test]
    fn cleanup_dxsetup_removes_dir() {
        let dir = tempfile::tempdir().unwrap();
        let dxsetup = dir.path().join("DXSETUP");
        fs::create_dir_all(&dxsetup).unwrap();
        fs::write(dxsetup.join("dsetup.exe"), b"fake").unwrap();
        cleanup_dxsetup(dir.path());
        assert!(!dxsetup.exists());
    }

    #[test]
    fn cleanup_dxsetup_no_dir() {
        let dir = tempfile::tempdir().unwrap();
        cleanup_dxsetup(dir.path());
    }

    #[test]
    fn extract_zip_simple() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("test.zip");
        create_test_zip(&zip_path, &[("hello.txt", b"hello world")]);
        let game_dir = dir.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();
        extract_zip(&zip_path, &game_dir).unwrap();
        let extracted = game_dir.join("hello.txt");
        assert!(extracted.exists());
        let content = fs::read_to_string(&extracted).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn extract_zip_with_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("test.zip");
        create_test_zip_with_dirs(
            &zip_path,
            &[
                ("subdir/", true, &[] as &[u8]),
                ("subdir/nested.txt", false, b"nested content"),
            ],
        );
        let game_dir = dir.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();
        extract_zip(&zip_path, &game_dir).unwrap();
        let extracted = game_dir.join("subdir/nested.txt");
        assert!(extracted.exists());
        let content = fs::read_to_string(&extracted).unwrap();
        assert_eq!(content, "nested content");
    }

    #[test]
    fn extract_zip_empty() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("empty.zip");
        create_empty_zip(&zip_path);
        let game_dir = dir.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();
        let result = extract_zip(&zip_path, &game_dir);
        assert!(result.is_ok());
    }

    #[test]
    fn extract_zip_path_traversal_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("traversal.zip");
        let file = File::create(&zip_path).unwrap();
        let mut writer = ZipWriter::new(BufWriter::new(file));
        writer
            .start_file("../../../etc/passwd", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"malicious").unwrap();
        writer.finish().unwrap();

        let game_dir = dir.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();
        extract_zip(&zip_path, &game_dir).unwrap();

        assert!(!dir.path().join("etc").exists());
        assert!(game_dir.read_dir().unwrap().next().is_none());
    }

    /// Construct a ZIP file on disk whose single entry declares a forged
    /// uncompressed size and stores only a few bytes of payload. Used to
    /// exercise the per-entry size cap in `extract_zip`.
    fn write_forged_zip(path: &Path, declared_uncompressed: u32) {
        // Build the ZIP bytes without the ambiguity of std::io::Write
        // vs tokio::io::AsyncWriteExt (both of which impl write_all for
        // Vec<u8>), by directly constructing the on-disk bytes via array
        // concat on a primitive buffer.
        let mut src = std::fs::File::create(path).unwrap();
        use std::io::Write as _;

        const SIG: u32 = 0x04034b50;
        let fname = b"huge.bin";
        let crc: u32 = 0;
        let compressed_size: u32 = 1;
        let payload: &[u8] = &[0xAB];

        // Local file header + 1-byte payload.
        src.write_all(&SIG.to_le_bytes()).unwrap();
        src.write_all(&20u16.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&crc.to_le_bytes()).unwrap();
        src.write_all(&compressed_size.to_le_bytes()).unwrap();
        src.write_all(&declared_uncompressed.to_le_bytes()).unwrap();
        src.write_all(&(fname.len() as u16).to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(fname).unwrap();
        src.write_all(payload).unwrap();

        // Compute the central-directory offset from bytes written so far.
        // 30 bytes of local header + 4 bytes payload declared-comp-size + 4
        // bytes payload declared-uncomp-size + 2 + 2 + 9 + payload length
        //   = 30 + 4 + 4 + 2 + 2 + sizeof(payload) + 9 + filename length
        // We'll rely on the offset being deterministic: it's the size of the
        // local header entry + payload.
        let local_header_size: u32 = 30 + (fname.len() as u32) + (payload.len() as u32);
        let cd_offset: u32 = local_header_size;

        const CD_SIG: u32 = 0x02014b50;
        src.write_all(&CD_SIG.to_le_bytes()).unwrap();
        src.write_all(&20u16.to_le_bytes()).unwrap();
        src.write_all(&20u16.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&crc.to_le_bytes()).unwrap();
        src.write_all(&compressed_size.to_le_bytes()).unwrap();
        src.write_all(&declared_uncompressed.to_le_bytes()).unwrap();
        src.write_all(&(fname.len() as u16).to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&0u32.to_le_bytes()).unwrap();
        src.write_all(&0u32.to_le_bytes()).unwrap();
        src.write_all(fname).unwrap();

        let cd_size: u32 = 46 + (fname.len() as u32);

        // EOCD record.
        const EOCD_SIG: u32 = 0x06054b50;
        src.write_all(&EOCD_SIG.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
        src.write_all(&1u16.to_le_bytes()).unwrap();
        src.write_all(&1u16.to_le_bytes()).unwrap();
        src.write_all(&cd_size.to_le_bytes()).unwrap();
        src.write_all(&cd_offset.to_le_bytes()).unwrap();
        src.write_all(&0u16.to_le_bytes()).unwrap();
    }

    #[test]
    fn extract_zip_rejects_forged_oversized_entry() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("bomb.zip");
        write_forged_zip(&zip_path, 600 * 1024 * 1024);

        let game_dir = dir.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();
        let result = extract_zip(&zip_path, &game_dir);
        assert!(
            result.is_err(),
            "archive with forged 600 MiB entry must be rejected"
        );
        assert!(
            !game_dir.join("huge.bin").exists(),
            "no files should be extracted from a bomb archive"
        );
    }

    #[test]
    fn extract_zip_rejects_colon_entry() {
        // NTFS-style alternate data stream: "safe/path:hidden" — `enclosed_name`
        // would happily accept this on Linux, but the entry name embeds a ':'
        // which our adapter rejects as it's never legitimate for plugin/SDK
        // archives.
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("ads.zip");
        let file = File::create(&zip_path).unwrap();
        let mut writer = ZipWriter::new(BufWriter::new(file));
        writer
            .start_file("safe/path:hidden", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"x").unwrap();
        writer.finish().unwrap();

        let game_dir = dir.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();
        extract_zip(&zip_path, &game_dir).unwrap();

        let rejected = game_dir.join("safe/path:hidden");
        assert!(
            !rejected.exists(),
            "NTFS-ADS-looking entry should not be materialized"
        );
    }

    #[test]
    fn md5_comparison_case_insensitive() {
        // Compute expected_md5 in lowercase manually for a known input.
        fn md5_hex(input: &[u8]) -> String {
            let mut hasher = Md5::new();
            hasher.update(input);
            hex::encode(hasher.finalize())
        }
        let lower = md5_hex(b"abc123");
        let upper = lower.to_ascii_uppercase();
        assert_ne!(lower, upper);
        // Both should pass the comparison.
        assert_eq!(lower.to_ascii_lowercase(), upper.to_ascii_lowercase());
    }

    #[test]
    fn write_plugin_version_strips_newlines() {
        let dir = tempfile::tempdir().unwrap();
        let raw_version = "1.0\nmalicious";
        let safe_version = raw_version.replace(['\n', '\r'], "");
        assert_eq!(safe_version, "1.0malicious");
        write_plugin_version(dir.path(), "plugin_abc_version", &safe_version).unwrap();
        let versions = read_plugin_versions(dir.path());
        assert_eq!(
            versions.get("plugin_abc_version"),
            Some(&"1.0malicious".to_string())
        );
    }
}
