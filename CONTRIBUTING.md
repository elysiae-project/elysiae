# Contributing to the Elysiae Launcher

The Elysiae Launcher is primarilly developed by two people new to the tools they are using, so if you identify issues with the source code, feel free to contribute! We will review your changes at our earliest convenience

## Setting up the development environment

Elyisae *must* run on a Linux system in order to successfully compile. If you are using Windows, consider using the [Windows Subsystem for Linux](https://aka.ms/wsl).

### Minimum System Requirements

To compile Elysiae, you should have the following present on your system:

1. A x86_64 or aarch64 CPU
2. Linux Kernel >= 6.14 (Recommended)
3. Rustup or an installation of Rust >= 1.98.1
4. FreeType >= 2.9.1
5. GTK >= 4.22
6. Any modern version of LLVM/Clang
7. A few gigabytes of free storage space for builds

### Installing Dependencies

#### Debian-Based

```sh
sudo apt update
sudo apt install libgtk-4-dev libfreetype-dev build-essential llvm clang -y

# Rustup might exist in a repo if you use a derivative of debian, but it is likely outdated. use curl to install the latest version instead:
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

#### Arch-Based

```sh
sudo pacman -Syu rustup freetype2 gtk4 base-devel llvm clang --noconfirm
```

## Post-clone

Make sure to install the toolchain used by Elysiae before doing anything else with the project:

```sh
# In the project dir
rustup toolchain install
```

## Creating Elysiae Builds

To create a developer build, run:

```sh
# Automatically runs the application
cargo run

# Alternatively, build and manually execute the developer build (binary will be created in ./target/debug)
cargo build
```

To create a release build, run:

```sh
# The compiled binary will be created in ./target/release
cargo build --release 

# Alternatively, you can run a release build directly from your terminal:
cargo run --release
```
