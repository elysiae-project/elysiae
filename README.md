# Elysiae

Universal "Chinese Anime Game" launcher for Linux

## Why does this project exist?

The Elysiae launcher was created in response to dissatisfaction in the state of the separated launchers and the departure of the initial goal of the unified "anime games" launcher created by [an-anime-team](https://github.com/an-anime-team). This project aims to deliver a minimal launcher build with many quality-of-life features, while trimming the fat where possible.

## Features

- Shared Proton runtime and prefix, with validated transactional runtime replacement and recovery.
- Game installation, updates, pre-downloads and integrity checks through the pinned irmin backend.
- Explicit installation ownership and backend APIs for import, verified relocation and managed-only deletion.
- Bounded localized media cache, English fallback and optional video backgrounds.
- Offline launch of installed games when a healthy runtime is available; online runtime checks do not automatically replace it.

Irmin's known pause/resume and repair limitations remain deferred upstream. Maintenance currently exposes Rust backend APIs, not new import/uninstall dialogs or CLI commands. Real-game compatibility and performance vary by game, runtime and hardware; the offline fixture suite does not establish either.

## Downloading Elysiae

> [!IMPORTANT]  
> Elysiae is not yet complete. Binaries will not be uploaded to package repositories until a stable 1.0 release is completed
> If you want to create a build yourself, you can follow the steps in [CONTRIBUTING](CONTRIBUTING.md)
