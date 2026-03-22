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
        let file = std::fs::File::open(&full_path).map_err(|e| e.to_string())?;
        let dest_path = std::path::Path::new(&dest);

        if full_path.ends_with(".tar.gz") {
            let decoder = Gz::new(file);
            let mut tar_archive = Tar::new(decoder);
            tar_archive.unpack(dest_path).map_err(|e| e.to_string())?;
        } else if full_path.ends_with(".tar.xz") {
            let decoder = Xz::new(file);
            let mut tar_archive = Tar::new(decoder);
            tar_archive.unpack(dest_path).map_err(|e| e.to_string())?;
        } else if full_path.ends_with(".tar.zst") {
            let decoder = Zstd::new(file).unwrap();
            let mut tar_archive = Tar::new(decoder);
            tar_archive.unpack(dest_path).map_err(|e| e.to_string())?;
        } else if full_path.ends_with(".zip") {
            let mut zip_archive = Zip::new(file).map_err(|e| e.to_string())?;
            zip_archive.extract(dest_path).map_err(|e| e.to_string())?;
        }

        // If the archive extracted into a single subdirectory, flatten it
        let top_level: Vec<_> = std::fs::read_dir(dest_path)
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;

        if top_level.len() == 1
            && top_level[0]
                .file_type()
                .map_err(|e| e.to_string())?
                .is_dir()
        {
            let subdir = top_level[0].path();
            for entry in std::fs::read_dir(&subdir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let target = dest_path.join(entry.file_name());
                std::fs::rename(entry.path(), &target).map_err(|e| e.to_string())?;
            }
            std::fs::remove_dir(&subdir).map_err(|e| e.to_string())?;
        }

        // Remove the archive now that extraction is done
        std::fs::remove_file(&full_path).map_err(|e| e.to_string())?;

        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[command]
pub fn get_all_files(path: &str, app_handle: AppHandle) -> Vec<String> {
    let full_path = app_handle
        .path()
        .resolve(&path, BaseDirectory::AppData)
        .unwrap();
    let base = app_handle
        .path()
        .resolve("", BaseDirectory::AppData)
        .unwrap();
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
    let base = app_handle
        .path()
        .resolve("", BaseDirectory::AppData)
        .unwrap();
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
    let base = app_handle
        .path()
        .resolve("", BaseDirectory::AppData)
        .unwrap();
    let mut items = vec![];
    for item in WalkDir::new(&full_path).min_depth(1).max_depth(1) {
        let entry = item.unwrap();
        let relative = entry.path().strip_prefix(&base).unwrap_or(entry.path());
        items.push(relative.display().to_string());
    }
    items
}
