
use tauri::{command};

#[command]
pub fn extract_file(archive: &str, destination: &str) {
    sevenz_rust::decompress_file(archive, destination).expect("Archive Extract Completed");
}
