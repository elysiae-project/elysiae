# Yoohoo!
Universal Hoyoverse game launcher using Wine, DXVK, And Proton

## Why This Project Exists

I created Yoohoo to replace the separated Hoyoverse game launchers from [an-anime-team](https://github.com/an-anime-team). From my experience, these launchers can be somewhat unreliable, are missing a few features, and are barely maintained. On top of that, each game requires its own launcher. While work has started on a [Unified Hoyoverse Launcher](https://github.com/an-anime-team/an-anime-game-launcher) several years ago, the project has largely stagnated, leading me to believe that the project will likely never be finished at all. These factors led me to creating Yoohoo, which attempts to do the following: 
    1. Function properly without any user intervention 
    2. Implement any features available on the Windows counterpart of Hoyoplay 
    3. Be actively maintained

## Software Requirements

To use Yoohoo properly, you will need the following:

- Linux x64 Host (Latest LTS Release)
- A DE running with the Wayland compositor (If you don't know what that is, you probably are running on Wayland)
- About 300Mb of storage
- Enough additional storage to install any Hoyoverse games you'd like to play

## Installing Yoohoo

You can install Yoohoo from flatpak, the AUR, or as a portable binary. Keep in mind that automatic updates will only work on flatpak and the AUR

## Building From Source / Contributing to Yoohoo

Please see [Contributing](https://github.com/shob3r/yoohoo/CONTRIBUTING.md)

## For the Hoyoverse lawyers

This app does NOT use any cheats, game plugins, or any other exploits to get Hoyoverse games running on Linux. It is running through Wine, a compatibility layer that allows for the execution of Windows binaries on Linux. Game rendering is performed through DXVK, another compatibility layer that translates DirectX shaders to Vulkan on run-time.

I am NOT hosting any game content myself, and am fetching it directly from your servers, like HoYoPlay does, and do not modify any files downloaded in this manner.

I also do not condone any user action that would result in a violation of your Terms of Service. I am merely providing a method for Linux users to play your games

Please look at the source code to verify my claims

## Disclaimer

while Yoohoo should be completely safe to use, please note that I am not responsible for any consequences that may come from using Yoohoo. Use at your own risk
