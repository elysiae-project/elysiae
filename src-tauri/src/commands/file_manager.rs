use flate2::read::GzDecoder as Gz;
use fs_extra::dir::get_size;
use std::io::Read;
use std::path::Path;
use tar::Archive as Tar;
use tauri::{AppHandle, Manager, command, path::BaseDirectory};
use xz::read::XzDecoder as Xz;
use zip::ZipArchive;
use zstd::Decoder as Zstd;

#[command]
pub async fn extract_file(
    archive: String,
    dest: String,
    app_handle: AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let full_path = app_handle
            .path()
            .resolve(&archive, BaseDirectory::AppData)
            .map_err(|err| {
                format!("extract_file: failed to resolve archive path for {archive}: {err}")
            })?;
        let full_dest = app_handle
            .path()
            .resolve(&dest, BaseDirectory::AppData)
            .map_err(|err| {
                format!("extract_file: failed to resolve dest path for {dest}: {err}")
            })?;

        if full_dest.exists() {
            std::fs::remove_dir_all(&full_dest).map_err(|err| {
                format!(
                    "extract_file: failed to remove existing dest {}: {err}",
                    full_dest.display()
                )
            })?;
        }

        if let Some(parent) = full_dest.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "extract_file: failed to create dest parent {}: {err}",
                    parent.display()
                )
            })?;
        }

        let path_str = full_path.to_string_lossy();

        let file = std::fs::File::open(&full_path).map_err(|err| {
            format!(
                "extract_file: failed to open archive {}: {err}",
                full_path.display()
            )
        })?;

        if path_str.ends_with(".tar.gz") {
            let decoder = Gz::new(file);
            let mut tar_archive = Tar::new(decoder);
            tar_archive.set_preserve_permissions(false);
            tar_archive.set_preserve_mtime(false);
            tar_archive.unpack(&full_dest).map_err(|err| {
                format!(
                    "extract_file: failed to unpack tar.gz to {}: {err}",
                    full_dest.display()
                )
            })?;
        } else if path_str.ends_with(".tar.xz") {
            let decoder = Xz::new(file);
            let mut tar_archive = Tar::new(decoder);
            tar_archive.set_preserve_permissions(false);
            tar_archive.set_preserve_mtime(false);
            tar_archive.unpack(&full_dest).map_err(|err| {
                format!(
                    "extract_file: failed to unpack tar.xz to {}: {err}",
                    full_dest.display()
                )
            })?;
        } else if path_str.ends_with(".tar.zst") {
            let decoder = Zstd::new(file).map_err(|err| err.to_string())?;
            let mut tar_archive = Tar::new(decoder);
            tar_archive.set_preserve_permissions(false);
            tar_archive.set_preserve_mtime(false);
            tar_archive.unpack(&full_dest).map_err(|err| {
                format!(
                    "extract_file: failed to unpack tar.zst to {}: {err}",
                    full_dest.display()
                )
            })?;
        } else if path_str.ends_with(".zip") {
            let mut zip_archive = ZipArchive::new(file).map_err(|err| err.to_string())?;
            extract_zip(&mut zip_archive, &full_dest)?;
        } else {
            return Err(format!("Unsupported archive format: {path_str}"));
        }

        flatten(&full_dest).map_err(|err| {
            format!(
                "extract_file: flatten failed for {}: {err}",
                full_dest.display()
            )
        })?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|err| err.to_string())?
}

fn extract_zip<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    dest: &Path,
) -> Result<(), String> {
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|err| err.to_string())?;
        let entry_path = match entry.enclosed_name() {
            Some(p) => p.to_owned(),
            None => continue,
        };
        let out_path = dest.join(&entry_path);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|err| {
                format!(
                    "extract_zip: failed to create dir {}: {err}",
                    out_path.display()
                )
            })?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|err| {
                    format!(
                        "extract_zip: failed to create parent {}: {err}",
                        parent.display()
                    )
                })?;
            }
            let mut out_file = std::fs::File::create(&out_path).map_err(|err| {
                format!(
                    "extract_zip: failed to create {}: {err}",
                    out_path.display()
                )
            })?;
            std::io::copy(&mut entry, &mut out_file).map_err(|err| {
                format!("extract_zip: failed to write {}: {err}", out_path.display())
            })?;
        }
    }
    Ok(())
}

#[command]
pub fn get_dir_size(path: String, app_handle: AppHandle) -> Result<u64, String> {
    let full_path = app_handle
        .path()
        .resolve(&path, BaseDirectory::AppData)
        .map_err(|err| format!("get_dir_size: failed to resolve path for {path}: {err}"))?;
    let dir_size = get_size(full_path).map_err(|err| err.to_string())?;

    Ok(dir_size)
}

fn flatten(dest: &Path) -> Result<(), String> {
    let entries: Vec<_> = std::fs::read_dir(dest)
        .map_err(|err| err.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e: std::io::Error| e.to_string())?;

    if entries.len() == 1 && entries[0].path().is_dir() {
        let inner_dir = entries[0].path();

        for entry in std::fs::read_dir(&inner_dir).map_err(|err| err.to_string())? {
            let entry = entry.map_err(|err| err.to_string())?;
            let target = dest.join(entry.file_name());
            if target.exists() {
                if target.is_dir() {
                    merge_dir(&entry.path(), &target)?;
                } else {
                    std::fs::remove_file(&target)
                        .or_else(|_| std::fs::remove_dir_all(&target))
                        .map_err(|err| err.to_string())?;
                    std::fs::rename(entry.path(), &target).map_err(|err| err.to_string())?;
                }
            } else {
                std::fs::rename(entry.path(), &target).map_err(|err| err.to_string())?;
            }
        }

        let remaining = std::fs::read_dir(&inner_dir)
            .map_err(|err| err.to_string())?
            .count();
        if remaining == 0 {
            std::fs::remove_dir(&inner_dir).map_err(|err| err.to_string())?;
        } else {
            log::warn!(
                "flatten: inner dir {} still has {} entries, not removing",
                inner_dir.display(),
                remaining
            );
        }
    }

    Ok(())
}

fn merge_dir(src: &Path, dest: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(src).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let target = dest.join(entry.file_name());
        if target.exists() {
            if target.is_dir() && entry.path().is_dir() {
                merge_dir(&entry.path(), &target)?;
            } else {
                std::fs::remove_file(&target)
                    .or_else(|_| std::fs::remove_dir_all(&target))
                    .map_err(|err| err.to_string())?;
                std::fs::rename(entry.path(), &target).map_err(|err| err.to_string())?;
            }
        } else {
            std::fs::rename(entry.path(), &target).map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}
