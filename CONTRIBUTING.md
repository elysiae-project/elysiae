# Contributing to the Elysiae Launcher

The Elysiae Launcher is primarilly developed by two people new to the tools they are using, so if you identify issues with the source code, feel free to contribute! We will review your changes at our earliest convenience

## Setting up the development environment

Elyisae *must* run on a Linux system in order to successfully compile. If you are using Windows, consider using the [Windows Subsystem for Linux](https://aka.ms/wsl).

### Minimum System Requirements

To compile Elysiae, you should have the following present on your system:

1. A x86_64 or aarch64 CPU
2. Linux Kernel >= 6.14 (Recommended)
3. Rustup or an installation of Rust >= 1.92.0
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

> [!IMPORTANT]  
> You must have an internet connection present to build Elysiae or have the appropriate rust version + cargo crates pre-downloaded before building

To create a developer build, run:

```sh
mkdir build && cd build
meson setup .. --buildtype=debug
ninja -j$(nproc) # Build with all threads
```

To create a release build, run:

```sh
mkdir build && cd build
meson setup .. --buildtype=release
ninja -j$(nproc)

# If you want to install:
ninja install -j$(nproc)
```

## Contribution Guidelines

On top of following the [Code of Conduct](https://github.com/elysiae-project/elysiae/CODE_OF_CONDUCT.md) , there are a few other general rules we'd like to have developers follow to ensure the Elysiae launcher is the best it can be:

### AI Usage

You are allowed to use AI tools to contribute to this project, so long as you review, test and understand the code (in other words, not "Vibe-coded"). Additionally, **ALL** contributors should be writing pull requests by hand to show that they understand what they are contributing. More information can be found in the [Code of Conduct](https://github.com/elysiae-project/elysiae/CODE_OF_CONDUCT.md)

### Dependencies and Rust/Meson versions

Generally, you are not allowed to add, update or remove crates or change rust versions except if:
1. You have a new feature that requires a new or more recent crate/rust version
2. You are writing a unit test that requires a new crate (must be added to dev-dependencies)
3. A version of a crate is out of support or has a major security vulnerability
 
If your contribution requires a rust/meson version change, you should update the relevant files (i.e. [rust-toolchain](https://github.com/elysiae-project/elysiae/rust-toolchain) and [CONTRIBUTING.md](https://github.com/elysiae-project/elysiae/CONTRIBUTING.md)) to reflect the new software requirements to build Elysiae

### Tests

You should always add unit tests to new functions you create, unless if they are very simple functions that you are certain cannot fail in any environment. As mentioned above, you are allowed to add rust crates (in dev-dependencies) and update the rust version if the crate requires a newer rust version

### Code Comments

Comments should reflect your understanding and explanations, rather than generic AI generated comments. Generic comments that state the obvious should be removed, rather than left in the code. Comments not in compliance with this policy will not cause your contribution to be rejected, but you may be asked to revise them if they are inappropriate.

### Commit Messages

Commit messages may be generated with AI assistance, so long as they are accurate description of the changes they describe.

### Pull Requests

Pull request descriptions must be written by the contributor, and should reflect their understanding and investigation of the changes requested.

### Issues

Issues must be written by the contributor, and should reflect their understanding and investigation of the issue.

### Review

Maintainers may ask you to explain any part of your contribution.

Your inability to adequately explain a change you submitted may be construed as failure to meet your responsibilities as a contributor, regardless of whether you used AI assisted development tools.
