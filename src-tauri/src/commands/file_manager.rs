use std::path::Path;
use walkdir::WalkDir;

use sha256::try_digest;
use tauri::{AppHandle, Manager, command, path::BaseDirectory};

#[command]
pub fn get_sha256_sum(path: String, app_handle: AppHandle) -> String {
    let full_path = app_handle
        .path()
        .resolve(&path, BaseDirectory::AppData)
        .unwrap();

    let input: &Path = Path::new(&full_path);
    try_digest(input).unwrap()
}

#[command]
pub fn get_all_files(path: &str, app_handle: AppHandle) -> Vec<String> {
    let full_path = app_handle
        .path()
        .resolve(&path, BaseDirectory::AppData)
        .unwrap();

    let mut files = vec![];
    for e in WalkDir::new(&full_path).into_iter().filter_map(Result::ok) {
        if e.metadata().unwrap().is_file() {
            files.push(e.path().display().to_string());
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

    let mut dirs = vec![];
    for e in WalkDir::new(&full_path).into_iter().filter_map(Result::ok) {
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
pub fn get_top_level_files(path: &str, app_handle: AppHandle) -> Vec<String> {
    let full_path = app_handle
        .path()
        .resolve(&path, BaseDirectory::AppData)
        .unwrap();

    let mut items = vec![];
    for item in WalkDir::new(&full_path).min_depth(1).max_depth(1) {
        let path = item.unwrap();
        items.push(path.path().display().to_string());
    }

    items
}
