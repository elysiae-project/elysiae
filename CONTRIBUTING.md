# Contributing To Yoohoo!

I welcome changes with open arms! I am a (relatively) new software developer and will make some mistakes that some of you can see from miles away. Please, feel free to submit a pull request if you know how to fix an issue/improve a feature

## Setting up the development environment

You **MUST** be running on a Linux system to have the program work. If you are developing on Windows, <u>use WSL</u>

Make sure you have all of the following build dependencies installed before attempting to build:

- rust >= 1.85.0 (Download via [rustup](https://rustup.rs))
- NodeJS >= 24.0.0
- Any dependencies listed on the [Tauri prerequisites page](https://tauri.app/start/prerequisites/)

After installing the system dependencies, install the NodeJS dependencies in the project:

> [!IMPORTANT]  
> Yoohoo uses the yarn package manager for NodeJS package management. Please ensure that corepack is installed and enabled by running ``npm i -g corepack@latest && corepack enable``. Doing so will download the appropriate version of ``yarn`` once you run the installation command below

```sh
yarn
```

### If you want to build flatpaks

Building flatpaks will require a bit of extra setup. You'll want to additionally install:

- Flatpak
- Flatpak Builder

You also want to install the flatpak Platforms/SDKs/Extensions that Yoohoo Uses:

```sh
flatpak install org.gnome.{Platform,Sdk}//48 -y
flatpak install org.freedesktop.Sdk.Extension.{node24,rust-stable}//24.08 -y
```

## Build Instructions

If you want to create a developer build, run:

```sh
yarn tauri dev # For A Development Build
```

If you want to create a release build, run:

```sh
yarn tauri build
```

if you want to generate a .flatpak file, run the following commands (in the source tree):

```sh
flatpak-builder --repo=repo --force-clean build dev.shob3r.yoohoo.yml
flatpak build-bundle repo dev.shob3r.yoohoo.flatpak dev.shob3r.yoohoo
```
