use std::fs;
use std::io::Read;
use std::path::Path;
use tauri::{AppHandle, Manager, command, path::BaseDirectory};
use walkdir::WalkDir;

use flate2::read::GzDecoder as Gz;
use xz::read::XzDecoder as Xz;
use zstd::Decoder as Zstd;

use tar::Archive as Tar;
use zip::ZipArchive as Zip;

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
            .unwrap();
        let full_dest = app_handle
            .path()
            .resolve(&dest, BaseDirectory::AppData)
            .unwrap();

        // Use the string representation for suffix checks
        let path_str = full_path.to_string_lossy();

        let file = std::fs::File::open(&full_path).map_err(|e| e.to_string())?;

        if path_str.ends_with(".tar.gz") {
            let decoder = Gz::new(file);
            let mut tar_archive = Tar::new(decoder);
            tar_archive.unpack(&full_dest).map_err(|e| e.to_string())?;
        } else if path_str.ends_with(".tar.xz") {
            let decoder = Xz::new(file);
            let mut tar_archive = Tar::new(decoder);
            tar_archive.unpack(&full_dest).map_err(|e| e.to_string())?;
        } else if path_str.ends_with(".tar.zst") {
            let decoder = Zstd::new(file).unwrap();
            let mut tar_archive = Tar::new(decoder);
            tar_archive.unpack(&full_dest).map_err(|e| e.to_string())?;
        } else if path_str.ends_with(".zip") {
            let mut zip_archive = Zip::new(file).map_err(|e| e.to_string())?;
            zip_archive.extract(&full_dest).map_err(|e| e.to_string())?;
        } else {
            return Err(format!("Unsupported archive format: {}", path_str));
        }

        flatten(&full_dest)?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn flatten(dest: &Path) -> Result<(), String> {
    let entries: Vec<_> = std::fs::read_dir(dest)
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e: std::io::Error| e.to_string())?;

    if entries.len() == 1 && entries[0].path().is_dir() {
        let inner_dir = entries[0].path();

        for entry in std::fs::read_dir(&inner_dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let target = dest.join(entry.file_name());
            std::fs::rename(entry.path(), &target).map_err(|e| e.to_string())?;
        }

        std::fs::remove_dir(&inner_dir).map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[command]
pub fn get_all_files(path: &str, app_handle: AppHandle) -> Vec<String> {
    let full_path = app_handle
        .path()
        .resolve(&path, BaseDirectory::AppData)
        .unwrap();
    let base = app_handle.path().app_data_dir().unwrap();
    let mut files = vec![];
    for e in WalkDir::new(&full_path).into_iter().filter_map(Result::ok) {
        if e.metadata().unwrap().is_file() {
            let relative = e.path().strip_prefix(&base).unwrap_or(e.path());
            files.push(relative.display().to_string());
        }
    }
    files
}

#[command]
pub fn get_all_directories(path: &str, app_handle: AppHandle) -> Vec<String> {
    let full_path = app_handle
        .path()
        .resolve(&path, BaseDirectory::AppData)
        .unwrap();
    let base = app_handle.path().app_data_dir().unwrap();

    let mut dirs = vec![];
    for e in WalkDir::new(&full_path).into_iter().filter_map(Result::ok) {
        if e.metadata().unwrap().is_dir() {
            if e.depth() == 0 {
                continue;
            }
            let relative = e.path().strip_prefix(&base).unwrap_or(e.path());
            dirs.push(relative.display().to_string());
        }
    }
    dirs
}

#[command]
pub fn get_top_level_files(path: &str, app_handle: AppHandle) -> Vec<String> {
    let full_path = app_handle
        .path()
        .resolve(&path, BaseDirectory::AppData)
        .unwrap();
    let base = app_handle.path().app_data_dir().unwrap();

    let mut items = vec![];
    for item in WalkDir::new(&full_path).min_depth(1).max_depth(1) {
        let entry = item.unwrap();
        let relative = entry.path().strip_prefix(&base).unwrap_or(entry.path());
        items.push(relative.display().to_string());
    }
    items
}

#[command]
pub fn get_md5_hash(path: &str, app_handle: AppHandle) -> Result<String, String> {
    let full_path = app_handle
        .path()
        .resolve(&path, BaseDirectory::AppData)
        .unwrap();

    let mut file = fs::File::open(full_path).map_err(|e| e.to_string())?;
    let mut buffer = Vec::new();
    let _ = file.read_to_end(&mut buffer).map_err(|e| e.to_string())?;

    let digest = md5::compute(&buffer);
    Ok(format!("{:x}", digest))
}