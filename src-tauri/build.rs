fn main() {
    #[cfg(not(target_os = "linux"))]
    compile_error!("Elysiae is only supported on Linux hosts.\nPlease build on a Linux environment");
    
    tauri_build::build()
}
