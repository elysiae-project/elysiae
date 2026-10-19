# Package the standalone bundle as well as Cargo's embedded resources. Keep
# build.rs independent so cargo build/run/test continue to work without Meson.
# gnome.compile_resources assigns the runtime installation tag to this bundle.
resources = gnome.compile_resources(
    'resources',
    '../resources.gresource.xml',
    source_dir: '..',
    gresource_bundle: true,
    install: true,
    install_dir: pkgdatadir,
)
