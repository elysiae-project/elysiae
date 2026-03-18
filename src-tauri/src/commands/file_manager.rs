use walkdir::WalkDir;
use tauri::{AppHandle, Manager, command, path::BaseDirectory};


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
