use flate2::read::GzDecoder as Gz;
use std::path::Path;
use tar::Archive as Tar;
use tauri::{AppHandle, Manager, command, path::BaseDirectory};
use xz::read::XzDecoder as Xz;
use zip::ZipArchive as Zip;
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
