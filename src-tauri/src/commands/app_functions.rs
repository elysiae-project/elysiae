use tauri::{command, is_dev};

#[command]
pub fn in_dev_env() -> bool {
    is_dev()
}
