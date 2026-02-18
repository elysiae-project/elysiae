use std::path::Path;

use sha256::try_digest;
use tauri::command;

#[command]
pub fn extract_file(archive: String, destination: String) {
    sevenz_rust::decompress_file(archive, destination).expect("Archive Extract Completed");
}

#[command]
pub fn get_sha256_sum(file: String) -> String {
    let input: &Path = Path::new(&file);
    try_digest(input).unwrap()
}
