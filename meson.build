# Based on the instructions on the gtk4 rust cookbook
project(
    'Elysiae', 'rust',
    version: '1.0.0',
    meson_version: '>= 1.4',
    default_options: ['buildtype=release'],
)

gnome = import('gnome')
base_id = 'app.elysiae.Elysiae'

if meson.is_cross_build()
    error('Meson cross builds are not supported; build natively on x86_64 or aarch64.')
endif

cargo_profile = get_option('profile')
buildtype = get_option('buildtype')
if cargo_profile == 'auto'
    if buildtype == 'debug'
        cargo_profile = 'dev'
    elif buildtype == 'release'
        cargo_profile = 'release'
    else
        error('Use --buildtype=debug/release, or explicitly select -Dprofile=dev/release. Cargo controls optimization settings.')
    endif
elif (cargo_profile == 'dev' and buildtype != 'debug') or (cargo_profile == 'release' and buildtype != 'release')
    warning('-Dprofile=' + cargo_profile + ' overrides Meson buildtype=' + buildtype + ' for Cargo. Use -Dprofile=auto to follow --buildtype.')
endif

if get_option('gui_tests') and not get_option('tests')
    error('-Dgui_tests=true requires -Dtests=true.')
endif
# Build profiles change optimization, not the running application's identity.
# Rust, GSettings, icons and URI activation all use this same stable ID.
profile = ''
application_id = base_id

bindir = get_option('prefix') / get_option('bindir')
localedir = get_option('prefix') / get_option('localedir')
datadir = get_option('prefix') / get_option('datadir')
pkgdatadir = datadir / meson.project_name()

subdir('data')

gsettings = custom_target(
    'compiled-gsettings',
    input: files('data/app.elysiae.Elysiae.gschema.xml'),
    output: 'gschemas.compiled',
    command: [
        find_program('glib-compile-schemas'),
        '--targetdir', meson.current_build_dir(),
        meson.current_source_dir() / 'data',
    ],
    build_by_default: true,
)

subdir('src')

gnome.post_install(
    gtk_update_icon_cache: true,
    glib_compile_schemas: true,
    update_desktop_database: true,
)
