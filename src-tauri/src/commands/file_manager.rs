use std::{fs::File, path::Path};

use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;
use walkdir::WalkDir;
use xz::read::XzDecoder;

use sha256::try_digest;
use tauri::command;

use tar::Archive as Tar;
use zip::ZipArchive as Zip;

#[command]
pub fn extract_file(archive: &str, destination: &str) {
    let file = File::open(archive).unwrap();

    if archive.ends_with(".tar.gz") {
        let decoder = GzDecoder::new(file);
        let mut tar_archive = Tar::new(decoder);
        tar_archive.unpack(destination).unwrap();
    } else if archive.ends_with(".tar.xz") {
        let decoder = XzDecoder::new(file);
        let mut tar_archive = Tar::new(decoder);
        tar_archive.unpack(destination).unwrap();
    } else if archive.ends_with("tar.bz2") {
        let decoder = BzDecoder::new(file);
        let mut tar_archive = Tar::new(decoder);
        tar_archive.unpack(destination).unwrap();
    } else if archive.ends_with(".tar") {
        let mut tar_archive = Tar::new(file);
        tar_archive.unpack(destination).unwrap();
    } else if archive.ends_with(".zip") {
        let mut zip_archive = Zip::new(file).unwrap();
        Zip::extract(&mut zip_archive, destination).unwrap();
    }
    // 7zip generates part archives where the first package in the part archive has the file extension .7z.001/.zip.001.
    // If there are more than 999 parts to the archive, it just adds an extra slot
    // TLDR: .7z.001/.zip.001 covers all part archives, unless they have been renamed
    else if archive.ends_with(".7z")
        || archive.ends_with(".7z.001")
        || archive.ends_with(".zip.001")
    {
        sevenz_rust::decompress(file, destination).unwrap();
    }
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
pub fn unwarp_root_folder() {
    
}