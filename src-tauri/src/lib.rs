use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;

use crate::commands::{app_functions, file_downloader, file_manager};
mod commands;
use crate::commands::sophon_downloader::ActiveDownload;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    apply_nvidia_wayland_workaround();

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .manage(commands::sophon_downloader::HttpClient(
            reqwest::Client::builder()
                .pool_max_idle_per_host(64)
                .build()
                .unwrap(),
        )) //  Required for sophon chunk downloading
        .manage(ActiveDownload(tokio::sync::Mutex::new(None)))
        .plugin(tauri_plugin_shell::init())
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
            file_downloader::download_file,
            file_manager::extract_file,
            app_functions::in_dev_env,
            commands::sophon_downloader::sophon_download,
            commands::sophon_downloader::sophon_update,
            commands::sophon_downloader::sophon_preinstall,
            commands::sophon_downloader::sophon_apply_preinstall,
            commands::sophon_downloader::sophon_pause,
            commands::sophon_downloader::sophon_resume,
            commands::sophon_downloader::sophon_cancel,
            commands::sophon_downloader::sophon_check_update,
        ])
        .setup(|app| {
            let quit_item = MenuItemBuilder::new("Quit Elysiae").id("quit").build(app)?;

            let menu = MenuBuilder::new(app).items(&[&quit_item]).build()?;

            TrayIconBuilder::new()
                .menu(&menu)
                .icon(app.default_window_icon().unwrap().clone())
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(target_os = "linux")]
fn apply_nvidia_wayland_workaround() {
    /*
     * webkit2gtk/webkit isn't implementing some of the the wayland compositor protocols
     * to the letter and NVIDIA drivers freak out because it expects implementations that do
     * follow the standards to the letter
     */
    if is_nvidia() && is_wayland() {
        println!("Elysiae: Applying NVIDIA Wayland Hotfix");
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
