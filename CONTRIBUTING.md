# Contributing To Elysiae

Elysiae is an application developed by only two people. We are both new to creating desktop applications with tauri and in general left a lot of room for improement down the road. If you are a capable developer and are willing to contribute improvements to Elysiae, we are welcoming your changes with open arms!

## Setting up the development environment

You **MUST** be running on a Linux system for Elysiae to successfully compile. If you are developing on Windows, use [WSL](https://aka.ms/wsl)

Make sure you have all of the following build dependencies installed before attempting to build:

- Rust >= 1.92.0 (Download via [rustup](https://rustup.rs))
- NodeJS >= 24.0.0
- Linux Kernel >= 6.14
- Systemd (Any recent version)
- A Desktop Environment running on Wayland
- Any dependencies listed on the [Tauri prerequisites page](https://tauri.app/start/prerequisites/)

After installing the system dependencies, install the NodeJS dependencies in the project:

> [!IMPORTANT]  
> Elysiae uses the yarn package manager for NodeJS package management. Please ensure that corepack is installed and enabled by running `npm i -g corepack@latest && corepack enable`. Doing so will download the appropriate version of `yarn` once you run the installation command below

```sh
yarn
```

### Flatpak Build Support

Generating flatpak installers will require a bit of extra setup. You'll want to additionally install:

- Flatpak
- Flatpak Builder

You also want to install the flatpak Platforms/SDKs/Extensions that Elysiae Uses:

```sh
flatpak install org.gnome.{Platform,Sdk}//49 -y
flatpak install org.freedesktop.Sdk.Extension.{node24,rust-stable}//25.08 -y
```

## Build Instructions

To run a developer build, run:

```sh
yarn tauri dev
```

To create a release build, run:

```sh
yarn tauri build
```

### Building Flatpak

```sh
flatpak-builder --force-clean --user --install-deps-from=flathub --repo=repo --install build app.elysiae.Elysiae.yml
flatpak build-bundle repo app.elysiae.Elysiae.flatpak app.elysiae.Elysiae
```
