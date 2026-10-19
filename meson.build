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
if host_machine.system() != 'linux' or host_machine.cpu_family() not in ['x86_64', 'aarch64']
    error('Elysiae requires Linux on x86_64 or aarch64.')
endif
if not meson.get_compiler('rust').version().version_compare('>=1.95.0')
    error('Elysiae requires Rust >= 1.95.0; install the toolchain specified by rust-toolchain.')
endif

# Match Cargo.toml's GTK v4_22/gnome_50 features and the direct font FFI.
# Cargo still owns linking; these checks make missing prerequisites fail early.
foreach requirement : [
    ['gtk4', '>=4.22'],
    ['glib-2.0', '>=2.88'],
    ['gio-2.0', '>=2.88'],
    ['pango', '>=1.52'],
    ['pangoft2', '>=1.52'],
    ['gdk-pixbuf-2.0', '>=2.42'],
    ['cairo', '>=1.16'],
    ['graphene-gobject-1.0', '>=1.10'],
]
    dependency(requirement[0], version: requirement[1], method: 'pkg-config')
endforeach
dependency('fontconfig', method: 'pkg-config')
find_program('glib-compile-resources')
find_program('xmllint') # Required by xml-stripblanks in the GResource manifest.
schema_compiler = find_program('glib-compile-schemas')
cargo = find_program('cargo')
shell = find_program('sh')
python = import('python').find_installation('python3')
if not run_command(cargo, '--version', check: true).stdout().split()[1].version_compare('>=1.95.0')
    error('Elysiae requires Cargo >= 1.95.0; use the pinned Rust toolchain.')
endif
if run_command(shell, '-c', 'printf "%s" "${CARGO_BUILD_TARGET:-}"', check: true).stdout() != ''
    error('CARGO_BUILD_TARGET is not supported by this native Meson build; unset it.')
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
if not get_option('tests') and get_option('validation').enabled()
    error('-Dvalidation=enabled requires -Dtests=true.')
endif
# Build profiles change optimization, not the running application's identity.
# Rust, GSettings, icons and URI activation all use this same stable ID.
application_id = base_id

bindir = get_option('prefix') / get_option('bindir')
datadir = get_option('prefix') / get_option('datadir')
pkgdatadir = datadir / meson.project_name()

subdir('data')

gsettings = custom_target(
    'compiled-gsettings',
    input: files('data/app.elysiae.Elysiae.gschema.xml'),
    output: 'gschemas.compiled',
    command: [
        schema_compiler,
        '--strict', '--targetdir', meson.current_build_dir(),
        meson.current_source_dir() / 'data',
    ],
    build_by_default: true,
)

subdir('src')

install_data(
    'LICENSE',
    install_dir: datadir / 'licenses' / application_id,
    install_tag: 'runtime',
)

gnome.post_install(
    gtk_update_icon_cache: true,
    glib_compile_schemas: true,
    update_desktop_database: true,
)
