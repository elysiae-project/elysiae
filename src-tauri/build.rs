fn main() {
    #[cfg(not(target_os = "linux"))]
    compile_error!(
        "Elysiae is only supported on Linux hosts. Please build in a Linux environment\nIf you are on Windows, You may want to consider taking a look at WSL: https://aka.ms/wsl"
    );

    #[cfg(not(target_arch = "x86_64"))]
    compile_error!(
        "Elysiae is only supported on x64 (x86_64) CPUs.\nIf you are running on an Arm CPU, you can target x64 and use FEX to run Elyisae: https://github.com/FEX-Emu/FEX\nFEX is untested not officially supported by the Elysiae Project; issues that occur because of its use will not be fixed."
    );

    tauri_build::build()
}
