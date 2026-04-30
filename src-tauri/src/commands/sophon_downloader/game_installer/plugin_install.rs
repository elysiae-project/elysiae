use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use bytes::BytesMut;
use futures_util::StreamExt;
use md5::{Digest, Md5};
use reqwest::Client;
use tauri_plugin_log::log;
use tokio::io::AsyncWriteExt;
use zip::ZipArchive;

use super::error::{SophonError, SophonResult};
use super::plugin_api::{
    ChannelSdkData, PackageData, PluginPackageInfo, ValidationEntry, fetch_channel_sdks,
    fetch_plugins, game_id_for_code,
};
use crate::commands::sophon_downloader::SophonProgress;

type ProgressFn = Arc<dyn Fn(SophonProgress) + Send + Sync>;

const PLUGIN_VERSIONS_FILE: &str = "plugin_versions.json";

fn read_plugin_versions(game_dir: &Path) -> HashMap<String, String> {
    let path = game_dir.join(PLUGIN_VERSIONS_FILE);
    let Ok(content) = fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn write_plugin_version(game_dir: &Path, key: &str, value: &str) -> std::io::Result<()> {
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
    let resp = client.get(url).send().await?.error_for_status()?;
    let total_bytes = resp.content_length().unwrap_or(0);

    let mut file = tokio::fs::File::create(dest).await?;
    let mut stream = resp.bytes_stream();
    let mut hasher = Md5::new();
    let mut downloaded: u64 = 0;

    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "plugin".to_string());

    let mut buffer = BytesMut::with_capacity(256 * 1024);

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        hasher.update(&bytes);
        buffer.extend_from_slice(&bytes);
        if buffer.len() >= 256 * 1024 {
            file.write_all(&buffer).await?;
            buffer.clear();
        }
        downloaded += bytes.len() as u64;
        updater(SophonProgress::DownloadingPlugin {
            name: name.clone(),
            downloaded_bytes: downloaded,
            total_bytes,
        });
    }

    if !buffer.is_empty() {
        file.write_all(&buffer).await?;
    }
    file.flush().await?;

    let actual_md5 = hex::encode(hasher.finalize());
    if actual_md5 != expected_md5 {
        let _ = fs::remove_file(dest);
        return Err(SophonError::Md5Mismatch {
            item: name,
            expected: expected_md5.to_string(),
            actual: actual_md5,
        });
    }

    Ok(())
}

fn extract_zip(zip_path: &Path, game_dir: &Path) -> SophonResult<()> {
    let file = File::open(zip_path)?;
    let reader = BufReader::new(file);
    let mut archive =
        ZipArchive::new(reader).map_err(|e| SophonError::Decompression(e.to_string()))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| SophonError::Decompression(e.to_string()))?;
        let out_path = match entry.enclosed_name() {
            Some(path) => game_dir.join(path),
            None => continue,
        };

        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out_file = File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out_file)?;
        }
    }

    Ok(())
}

fn cleanup_dxsetup(game_dir: &Path) {
    let dxsetup_dir = game_dir.join("DXSETUP");
    if dxsetup_dir.is_dir()
        && let Err(e) = fs::remove_dir_all(&dxsetup_dir)
    {
        log::warn!("Failed to clean up DXSETUP directory: {}", e);
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

    let filename = pkg
        .url
        .rsplit('/')
        .next()
        .unwrap_or("plugin.zip")
        .to_string();
    let zip_path = game_dir.join(&filename);

    download_zip(client, &pkg.url, &zip_path, &pkg.md5, updater).await?;

    if let Err(e) = extract_zip(&zip_path, game_dir) {
        let _ = fs::remove_file(&zip_path);
        return Err(e);
    }

    if !verify_validation(game_dir, &pkg.validation) {
        let _ = fs::remove_file(&zip_path);
        return Err(SophonError::PluginValidationFailed(
            plugin.plugin_id.clone(),
        ));
    }

    cleanup_dxsetup(game_dir);

    let safe_version = plugin.version.replace(['\n', '\r'], "");
    write_plugin_version(
        game_dir,
        &format!("plugin_{}_version", plugin.plugin_id),
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

        if let Err(e) =
            install_single_plugin(client, game_dir, plugin, &plugin.plugin_pkg, &updater).await
        {
            log::warn!("Plugin {} installation failed: {}", plugin.plugin_id, e);
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

    let filename = sdk
        .channel_sdk_pkg
        .url
        .rsplit('/')
        .next()
        .unwrap_or("sdk.zip")
        .to_string();
    let zip_path = game_dir.join(&filename);

    download_zip(
        client,
        &sdk.channel_sdk_pkg.url,
        &zip_path,
        &sdk.channel_sdk_pkg.md5,
        updater,
    )
    .await?;

    if let Err(e) = extract_zip(&zip_path, game_dir) {
        let _ = fs::remove_file(&zip_path);
        return Err(e);
    }

    if !verify_validation(game_dir, &sdk.channel_sdk_pkg.validation) {
        let _ = fs::remove_file(&zip_path);
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
        updater(SophonProgress::InstallingPlugins {
            current_plugin: format!("sdk_{}", sdk.game.id),
            total_plugins: total,
        });

        if let Err(e) = install_single_sdk(client, game_dir, sdk, &updater).await {
            log::warn!("SDK {} installation failed: {}", sdk.game.id, e);
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
        let config = dir.path().join(CONFIG_INI);
        fs::write(
            &config,
            "[General]\nplugin_abc_version=1.0\nplugin_def_version=2.0\n",
        )
        .unwrap();
        let versions = read_plugin_versions(dir.path());
        assert_eq!(versions.len(), 2);
        assert_eq!(versions.get("plugin_abc_version"), Some(&"1.0".to_string()));
        assert_eq!(versions.get("plugin_def_version"), Some(&"2.0".to_string()));
    }

    #[test]
    fn read_plugin_versions_stops_at_next_section() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(CONFIG_INI);
        fs::write(
            &config,
            "[General]\nplugin_abc_version=1.0\n[Other]\nplugin_def_version=2.0\n",
        )
        .unwrap();
        let versions = read_plugin_versions(dir.path());
        assert_eq!(versions.len(), 1);
        assert_eq!(versions.get("plugin_abc_version"), Some(&"1.0".to_string()));
        assert!(versions.get("plugin_def_version").is_none());
    }

    #[test]
    fn write_plugin_version_new_file() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin_version(dir.path(), "plugin_abc_version", "1.0").unwrap();
        let content = fs::read_to_string(dir.path().join(CONFIG_INI)).unwrap();
        assert!(content.contains("[General]"));
        assert!(content.contains("plugin_abc_version=1.0"));
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

    #[test]
    fn write_plugin_version_strips_newlines() {
        let dir = tempfile::tempdir().unwrap();
        let raw_version = "1.0\nmalicious";
        let safe_version = raw_version.replace(['\n', '\r'], "");
        assert_eq!(safe_version, "1.0malicious");
        write_plugin_version(dir.path(), "plugin_abc_version", &safe_version).unwrap();
        let content = fs::read_to_string(dir.path().join(CONFIG_INI)).unwrap();
        assert!(content.contains("plugin_abc_version=1.0malicious\n"));
        for line in content.lines() {
            if line.starts_with("plugin_abc_version=") {
                assert_eq!(line, "plugin_abc_version=1.0malicious");
            }
        }
        let versions = read_plugin_versions(dir.path());
        assert_eq!(
            versions.get("plugin_abc_version"),
            Some(&"1.0malicious".to_string())
        );
    }
}
