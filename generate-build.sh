# This script is a work-in-progress and is the intended way to build for release.
# I plan on adding automatic dependency installation (both from os package manager and the flatpak SDKs) in the near future
# For now, install:
# flatpak (os pkg manager)
# flatpak-builder (os pkg manager)
# org.gnome.Platform//48 (flatpak)
# org.gnome.Sdk//48 (flatpak)
# org.freedesktop.Sdk.Extension.node20//24.08 (flatpak)
# org.freedesktop.Sdk.Extension.rust-stable//24.08 (flatpak)

export RUSTFLAGS="-C target-cpu=native" 

flatpak-builder --repo=repo --force-clean build net.shob3r.yoohoo.yml
flatpak build-bundle repo net.shob3r.yoohoo.flatpak net.shob3r.yoohoo
rm -rf repo
rm -rf .flatpak-builder
rm -rf build

unset RUSTFLAGS
