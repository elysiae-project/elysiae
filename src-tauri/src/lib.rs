mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    apply_nvidia_wayland_workaround();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {}))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::file_downloader::download_file,
            commands::file_manager::extract_file,
            commands::file_manager::get_all_directories,
            commands::file_manager::get_all_files,
            commands::file_manager::get_top_level_files,
            commands::app_functions::in_dev_env,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(target_os = "linux")]
fn apply_nvidia_wayland_workaround() {
    /* Extensive digging has revealed why this workaround is needed on NVIDIA devices (from my understanding):
     *
     * webkit2gtk isn't implementing some of the the wayland compositor protocols
     * to the letter and NVIDIA drivers freak out because it expects implementations that do
     * follow the standards to the letter
     */
    if is_nvidia() && is_wayland() {
        println!("Applying NVIDIA Wayland Hotfix");
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
}

#[cfg(target_os = "linux")]
fn is_nvidia() -> bool {
    // If a NVIDIA graphics card is present, one of these two files should also be available
    std::path::Path::new("/proc/driver/nvidia/version").exists()
        || std::path::Path::new("/dev/nvidia0").exists()
}

#[cfg(target_os = "linux")]
fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.to_lowercase() == "wayland")
            .unwrap_or(false)
}
