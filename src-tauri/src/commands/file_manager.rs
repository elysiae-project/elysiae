use std::path::Path;
use walkdir::WalkDir;

use sha256::try_digest;
use tauri::command;


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
