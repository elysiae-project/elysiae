use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::constants::VERSION_FILE_NAME;

fn version_file_path(game_dir: &Path) -> PathBuf {
    game_dir.join(VERSION_FILE_NAME)
}

pub fn read_installed_tag(game_dir: &Path) -> Option<String> {
    fs::read_to_string(version_file_path(game_dir))
        .ok()
        .map(|s| s.trim().to_owned())
}

pub fn write_installed_tag(game_dir: &Path, tag: &str) -> io::Result<()> {
    fs::write(version_file_path(game_dir), tag)
}
