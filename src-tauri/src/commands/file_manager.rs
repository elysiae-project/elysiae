use std::{fs::File, path::Path};

use bzip2::read::BzDecoder as Bz2;
use flate2::read::GzDecoder as Gz;
use walkdir::WalkDir;
use xz::read::XzDecoder as Xz;
use zstd::Decoder as Zstd;

use sha256::try_digest;
use tauri::command;

use tar::Archive as Tar;
use zip::ZipArchive as Zip;

#[command]
pub async fn extract_file(archive: String, destination: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let file = std::fs::File::open(&archive).map_err(|e| e.to_string())?;

        if archive.ends_with(".tar.gz") {
            let decoder = Gz::new(file);
            let mut tar_archive = Tar::new(decoder);
            tar_archive
                .unpack(&destination)
                .map_err(|e| e.to_string())?;
        } else if archive.ends_with(".tar.xz") {
            let decoder = Xz::new(file);
            let mut tar_archive = Tar::new(decoder);
            tar_archive
                .unpack(&destination)
                .map_err(|e| e.to_string())?;
        } else if archive.ends_with(".tar.bz2") {
            let decoder = Bz2::new(file);
            let mut tar_archive = Tar::new(decoder);
            tar_archive
                .unpack(&destination)
                .map_err(|e| e.to_string())?;
        } else if archive.ends_with(".tar.zst") {
            let decoder = Zstd::new(file).unwrap();
            let mut tar_archive = Tar::new(decoder);
            tar_archive
                .unpack(&destination)
                .map_err(|e| e.to_string())?;
        } else if archive.ends_with(".tar") {
            let mut tar_archive: Tar<File> = Tar::new(file);
            tar_archive
                .unpack(&destination)
                .map_err(|e| e.to_string())?;
        } else if archive.ends_with(".zip") {
            let mut zip_archive = Zip::new(file).map_err(|e| e.to_string())?;
            zip_archive
                .extract(&destination)
                .map_err(|e| e.to_string())?;
        } else if archive.ends_with(".7z")
            || archive.ends_with(".7z.001")
            || archive.ends_with(".zip.001")
        {
            sevenz_rust::decompress(file, &destination).map_err(|e| e.to_string())?;
        }

        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[command]
pub fn get_sha256_sum(file: String) -> String {
    let input: &Path = Path::new(&file);
    try_digest(input).unwrap()
}

#[command]
pub fn get_all_files(path: &str) -> Vec<String> {
    let mut files = vec![];
    for e in WalkDir::new(path).into_iter().filter_map(Result::ok) {
        if e.metadata().unwrap().is_file() {
            files.push(e.path().display().to_string());
        }
    }
    files
}

#[command]
pub fn get_all_directories(path: &str) -> Vec<String> {
    let mut dirs = vec![];
    for e in WalkDir::new(path).into_iter().filter_map(Result::ok) {
        if e.metadata().unwrap().is_dir() {
            if e.depth() == 0 {
                continue;
            }
            dirs.push(e.path().display().to_string());
        }
    }
    dirs
}

#[command]
pub fn get_top_level_files(path: &str) -> Vec<String> {
    let mut items = vec![];
    for item in WalkDir::new(path).min_depth(1).max_depth(1) {
        let path = item.unwrap();
        items.push(path.path().display().to_string());
    }

    items
}
