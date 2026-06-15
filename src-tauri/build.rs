fn main() {
    #[cfg(not(target_os = "linux"))]
    compile_error!(
        "Elysiae is only supported on Linux hosts. Please build in a Linux environment\nIf you are on Windows, You may want to consider taking a look at WSL: https://aka.ms/wsl"
    );

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    compile_error!(
        "Elysiae is only supported on x64 (x86_64) and ARM (aarch64) computers. Please target one of these two architectures when building Elysiae"
    );

    if !kernel_version_at_least_6_14_0() {
        eprintln!(
            "Elysiae is only supported on Linux Kernel version 6.14 or later. Please run a system update before attempting to compile Elysiae again"
        );
        std::process::exit(1);
    }

    tauri_build::build()
}

fn kernel_version_at_least_6_14_0() -> bool {
    if std::env::var("ELYSIAE_SKIP_KERNEL_CHECK").is_ok() {
        return true;
    }
    let release = match std::fs::read_to_string("/proc/sys/kernel/osrelease") {
        Ok(s) => s,
        Err(_) => return false,
    };

    version_at_least(&release, (6, 14, 0))
}

fn version_at_least(release: &str, min: (u64, u64, u64)) -> bool {
    let mut parts = release
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<u64>().ok());

    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);

    (major, minor, patch) >= min
}
