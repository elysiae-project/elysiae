use crate::commands::{file_downloader, file_manager};
mod commands;
use crate::commands::sophon_downloader::ActiveDownload;
use std::env;
use tauri::command;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    apply_nvidia_wayland_workaround();

    #[cfg(target_os = "linux")]
    apply_webkit_memory_improvements();

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .manage(commands::sophon_downloader::HttpClient(
            reqwest::Client::builder()
                .pool_max_idle_per_host(64)
                .user_agent(format!(
                    "{}/{}",
                    env!("CARGO_PKG_NAME"),
                    env!("CARGO_PKG_VERSION")
                ))
                .build()
                .unwrap(),
        )) // Required for sophon chunk downloading
        .manage(ActiveDownload(tokio::sync::Mutex::new(None)))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                window.unminimize().ok();
                window.set_focus().ok();
            }
        }))        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(disable_shortcuts())
        .setup(|_app| {
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            file_downloader::download_file,
            file_manager::extract_file,
            file_manager::get_dir_size,
            commands::sophon_downloader::sophon_download,
            commands::sophon_downloader::sophon_update,
            commands::sophon_downloader::sophon_preinstall,
            commands::sophon_downloader::sophon_apply_preinstall,
            commands::sophon_downloader::sophon_resume_download,
            commands::sophon_downloader::sophon_has_resume_state,
            commands::sophon_downloader::sophon_get_resume_info,
            commands::sophon_downloader::sophon_verify_integrity,
            commands::sophon_downloader::sophon_pause,
            commands::sophon_downloader::sophon_resume,
            commands::sophon_downloader::sophon_cancel,
            commands::sophon_downloader::sophon_check_update,
            elysiae_version,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(target_os = "linux")]
fn apply_webkit_memory_improvements() {
    unsafe {
        std::env::set_var("WEBKIT_FORCE_MEMORY_PRESSURE_SYSTEM", "critical");
        std::env::set_var("WEBKIT_CACHE_MODEL", "document_viewer");
    }
}

#[cfg(target_os = "linux")]
fn apply_nvidia_wayland_workaround() {
    if is_nvidia() && is_wayland() {
        println!("Elysiae: Applying NVIDIA Wayland Workaround");
        unsafe {
            std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
            std::env::set_var("WEBKIT_DISABLE_GPU_COMPOSITING", "1");
            std::env::set_var("WEBKIT_DISABLE_VAAPI", "1");
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        };
    }
}

#[cfg(target_os = "linux")]
fn is_nvidia() -> bool {
    // If a NVIDIA graphics card is present, one of these two paths should exist
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

#[cfg(debug_assertions)]
fn disable_shortcuts() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_prevent_default::Flags;

    tauri_plugin_prevent_default::Builder::new()
        .with_flags(Flags::empty())
        .build()
}

#[cfg(not(debug_assertions))]
fn disable_shortcuts() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_prevent_default::Flags;

    tauri_plugin_prevent_default::Builder::new()
        .with_flags(Flags::all())
        .build()
}

#[command]
fn elysiae_version() -> String {
    env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "Unknown App Version".to_string())
}
