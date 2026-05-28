use std::env;

use tauri::{command, is_dev};

#[command]
pub fn in_dev_env() -> bool {
    is_dev()
}

#[command]
pub fn get_app_version() -> String {
    env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "Unknown App Version".to_string())
}
