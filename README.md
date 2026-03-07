# Yoohoo

<p align="center">
<img src="https://raw.githubusercontent.com/Shob3r/yoohoo/refs/heads/main/app-icon.png" alt="Yoohoo App Icon" width="256">
</p>

> [!WARNING]  
> This project is in Alpha! Some features are missing and others may have issues.

## Why This Project Exists

Yoohoo aims to replace the separated game launchers from [an-anime-team](https://github.com/an-anime-team). From my experience, these launchers can be somewhat unreliable, are missing a few features, and are barely maintained. On top of that, each game requires its own launcher. While work has started on a [Unified Launcher](https://github.com/an-anime-team/anime-games-launcher) several years ago, the project has largely stagnated, leading me to believe that the project will likely never be finished at all. These factors led me to creating Yoohoo, which attempts to do the following:

1. Function without any user intervention
2. Implement any features available on the Windows counterpart of the launcher
3. Be actively maintained

## Software Requirements

To use Yoohoo, you will need the following:

- A 64-Bit CPU
- Linux Kernel >=6.1 with support for futex2 (Most distros will have this)
- systemd (Most Linux installs. If you don't know what this means, you have this)
- A Desktop Environment running on the Wayland compositor (You most likely have this)
- About 300Mb of storage
- Enough additional storage to install any of the games you'd like to play

## Installing Yoohoo

**WIP**

## Building From Source / Contributing to Yoohoo

Please see [Contributing](https://github.com/shob3r/yoohoo/CONTRIBUTING.md)

## For the lawyers of the "anime game" company

This app does **NOT** use any cheats/game plugins or any other exploits to get Hoyoverse games running on Linux. It is running through Wine, a compatibility layer that allows for the execution of Windows binaries on Linux. Game rendering is performed through DXVK, another compatibility layer that translates DirectX shaders to Vulkan on run-time.

I am NOT hosting any game content myself, and am fetching it directly from your servers, like HoYoPlay does, and do not modify any files downloaded in this manner.

I also do not condone any user action that would result in a violation of your Terms of Service. I am merely providing a method for Linux users to play your games

Please look at the source code to verify my claims

## Disclaimer

Yoohoo should be completely safe to use. However, please note that I am not responsible for any consequences that may come from using Yoohoo. Use this program at your own risk
