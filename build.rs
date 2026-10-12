#[cfg(not(test))]
fn main() {
    println!("cargo:rerun-if-env-changed=ELYSIAE_SKIP_KERNEL_CHECK");
    compile_resources();
    let status = std::process::Command::new("glib-compile-schemas")
        .args(["--strict", "--targetdir"])
        .arg(std::env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"))
        .arg("data")
        .status()
        .expect("glib-compile-schemas is required to build Elysiae");
    assert!(status.success(), "GSettings schema compilation failed");

    #[cfg(not(target_os = "linux"))]
    compile_error!(
        "Elysiae is only supported on Linux hosts. Please build in a Linux environment\nIf you are on Windows, You may want to consider taking a look at WSL: https://aka.ms/wsl"
    );

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    compile_error!(
        "Elysiae is only supported on x64 (x86_64) and ARM (aarch64) computers. Please target one of these two architectures when building Elysiae.\nWARNING: If you remove this warning, the app will build successfully, but will fail to install the required additional files, as the endpoint used to download the additional files only provides arm/x86 binaries".
    );

    if !supported_kernel() {
        println!(
            "cargo:warning=Elysiae is designed to run on Linux kernel version 6.14 or later. Please update the kernel if you encounter performance issues while playing games"
        );
    }
}

/// Compiles the gresource bundle into OUT_DIR (picked up by
/// `gio::resources_register_include!`) following the gtk4-rs book pattern.
#[cfg(not(test))]
fn compile_resources() {
    println!("cargo:rerun-if-changed=data/resources.gresource.xml");
    println!("cargo:rerun-if-changed=data");
    glib_build_tools::compile_resources(
        &["data"],
        "data/resources.gresource.xml",
        "elysiae.gresource",
    );
}

#[cfg(not(test))]
fn supported_kernel() -> bool {
    if std::env::var_os("ELYSIAE_SKIP_KERNEL_CHECK").is_some() {
        return true;
    }
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .is_ok_and(|release| version_at_least(&release, (6, 14, 0)))
}

fn version_at_least(release: &str, minimum: (u64, u64, u64)) -> bool {
    let mut parts = release
        .trim()
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok());
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    ) >= minimum
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_kernel_versions() {
        assert!(version_at_least("6.14.0", (6, 14, 0)));
        assert!(version_at_least("6.15.2-arch1", (6, 14, 0)));
        assert!(!version_at_least("6.13.9", (6, 14, 0)));
        assert!(version_at_least("6.14-foo", (6, 14, 0)));
        assert!(!version_at_least("invalid", (6, 14, 0)));
    }
}
