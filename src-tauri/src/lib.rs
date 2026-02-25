mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
        .plugin(tauri_plugin_upload::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::file_manager::get_sha256_sum,
            // commands::file_manager::extract_file,
            commands::file_downloader::download_file,
            commands::file_manager::get_all_directories,
            commands::file_manager::get_all_files,
            commands::file_manager::get_top_level_files,
            commands::app_functions::in_dev_env,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
