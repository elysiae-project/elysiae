set -e
cd "$(dirname "$0")"

echo "(1/7) Checking for system dependencies"
for command in flatpak flatpak-builder node npm; do 
    if ! $command --version >/dev/null 2>&1
    then
        echo "$command does not appear to be installed on your system"
        echo "Please install all required build dependencies before running again"
        exit 1
    fi
done

echo "(2/7) Checking for & installing missing flatpak build dependencies"
for build_dep in org.gnome.Platform//48 org.gnome.Sdk//48 org.freedesktop.Sdk.Extension.node24//25.08 org.freedesktop.Sdk.Extension.rust-stable//25.08; do
    if ! $build_dep --version >/dev/null 2>&1
    then
        echo "Installing build dependency $build_dep"
        flatpak install -y flathub $build_dep
    fi
done

echo "(3/7) Preparing frontend dependency install"
if ! corepack --version >/dev/null 2>&1
then
    echo "Installing corepack"
    npm i -g corepack
    corepack enable
fi

# Yes, this should be done inside the flatpak environment, but several issues arose while I was trying to implement that
# 1. Yarn 1.22.0 in flatpak build env only
# 2. Even when I did figure out how to get it to work, it really did not want to
# 3. This is flat-out easier in my opinion
# Rust dependencies are downloaded in the flatpak environment, but only because it isn't cancerous to do so
# In the future, I might have the flatpak manifest handle this, but for now, this script will
echo "(4/7) Installing frontend dependencies"
yarn

echo "(5/7) Building flatpak app"
flatpak-builder --repo=repo --force-clean build net.shob3r.yoohoo.yml

echo "(6/7) Bundling flatpak app"
flatpak build-bundle repo net.shob3r.yoohoo.flatpak net.shob3r.yoohoo


# A single & means "Run the program/command, but the shell shouldn't wait for it to finish"
# There's no need to wait for cleanup to complete, so it's being used here
echo "(7/7) Cleanup"
rm -rf {repo,.flatpak-builder,build} & >/dev/null

echo "Done! .flatpak file located in $PWD/net.shob3r.yoohoo.flatpak"
