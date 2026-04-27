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
    PackageData, PluginPackageInfo, ValidationEntry, fetch_channel_sdks, fetch_plugins,
    game_id_for_code,
};
use crate::commands::sophon_downloader::SophonProgress;

type ProgressFn = Arc<dyn Fn(SophonProgress) + Send + Sync>;

const CONFIG_INI: &str = "config.ini";
const GENERAL_SECTION: &str = "[General]";

fn read_plugin_versions(game_dir: &Path) -> HashMap<String, String> {
    let config_path = game_dir.join(CONFIG_INI);
    let Ok(content) = fs::read_to_string(&config_path) else {
        return HashMap::new();
    };

    let mut versions = HashMap::new();
    let mut in_general = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == GENERAL_SECTION {
            in_general = true;
            continue;
        }
        if in_general && trimmed.starts_with('[') {
            break;
        }
        if in_general && let Some((key, value)) = trimmed.split_once('=') {
            versions.insert(key.trim().to_string(), value.trim().to_string());
        }
    }

    versions
}

fn write_plugin_version(game_dir: &Path, key: &str, value: &str) -> std::io::Result<()> {
    let config_path = game_dir.join(CONFIG_INI);

    if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        let mut lines: Vec<String> = content.lines().map(String::from).collect();
        let mut in_general = false;
        let mut found = false;

        for line in &mut lines {
            let trimmed = line.trim();
            if trimmed == GENERAL_SECTION {
                in_general = true;
                continue;
            }
            if in_general && trimmed.starts_with('[') {
                in_general = false;
            }
            if in_general
                && trimmed.starts_with(key)
                && let Some(eq_pos) = trimmed.find('=')
                && trimmed[..eq_pos].trim() == key
            {
                *line = format!("{key}={value}");
                found = true;
            }
        }

        if !found {
            let mut insert_idx = 0;
            let mut in_general_section = false;
            for (i, line) in lines.iter().enumerate() {
                if line.trim() == GENERAL_SECTION {
                    in_general_section = true;
                    insert_idx = i + 1;
                } else if in_general_section && line.trim().starts_with('[') {
                    insert_idx = i;
                    break;
                } else if in_general_section {
                    insert_idx = i + 1;
                }
            }
            if in_general_section {
                lines.insert(insert_idx, format!("{key}={value}"));
            } else {
                lines.push(String::new());
                lines.push(GENERAL_SECTION.to_string());
                lines.push(format!("{key}={value}"));
            }
        }

        let mut out = lines.join("\n");
        if !out.ends_with('\n') {
            out.push('\n');
        }
        fs::write(&config_path, out)
    } else {
        let content = format!("{GENERAL_SECTION}\n{key}={value}\n");
        fs::write(&config_path, content)
    }
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

    verify_validation(game_dir, &pkg.validation);

    cleanup_dxsetup(game_dir);

    write_plugin_version(
        game_dir,
        &format!("plugin_{}_version", plugin.plugin_id),
        &plugin.version,
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

        install_single_plugin(client, game_dir, plugin, &plugin.plugin_pkg, &updater).await?;
    }

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

        let key = "plugin_sdk_version";
        let versions = read_plugin_versions(game_dir);

        if versions.get(key).map(String::as_str) == Some(sdk.version.as_str())
            && verify_validation(game_dir, &sdk.channel_sdk_pkg.validation)
        {
            continue;
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
            &updater,
        )
        .await?;

        if let Err(e) = extract_zip(&zip_path, game_dir) {
            let _ = fs::remove_file(&zip_path);
            return Err(e);
        }

        verify_validation(game_dir, &sdk.channel_sdk_pkg.validation);

        cleanup_dxsetup(game_dir);

        write_plugin_version(game_dir, key, &sdk.version)?;

        let _ = fs::remove_file(&zip_path);
    }

    Ok(())
}
