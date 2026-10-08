# Elysiae

Universal "Chinese Anime Game" launcher for Linux

## Why does this project exist?

The Elysiae launcher was created in response to dissatisfaction in the state of the separated launchers and the departure of the initial goal of the unified "anime games" launcher created by [an-anime-team](https://github.com/an-anime-team). This project aims to deliver a minimal launcher build with many quality-of-life features, while trimming the fat where possible.

## Features

- Unified Proton install: If you play multiple "Anime Games", this will save you ~350-450MB per extra game you play (compared to the individual launchers from an-anime-team)
- Low-profile: Elysiae has been built to behave as a thin wrapper around the games it launches. Install size is low (< 1mb binary size) and memory usage has been reduced as much as possible (during game downloads, Elysiae will only use ~400MB of system memory)
- Fast installs: Our game downloader library, irmin, has been optimized to download games faster than any other "Anime Game" launcher available. On gigabit networks, game downloads on elysiae are about 6x-10x faster than on projects created by an-anime-team

## Downloading Elysiae

> [!IMPORTANT]  
> Elysiae is not yet complete. Binaries will not be uploaded to package repositories until a stable 1.0 release is completed
> If you want to create a build yourself, you can follow the steps in [CONTRIBUTING](https://github.com/elysiae-project/elysiae/CONTRIBUTING.md)
