fs = import('fs')

# Cargo owns dependency tracking and profile settings, so this file only
# invokes it and installs the resulting binary.
bin_name = meson.project_name()

# Relative paths resolve against the source root. Set -Dcargo_target_dir to
# share a cache between build directories.
cargo_target_dir = get_option('cargo_target_dir')
if cargo_target_dir == ''
    cargo_target_dir = meson.project_build_root() / 'target'
elif not fs.is_absolute(cargo_target_dir)
    cargo_target_dir = meson.project_source_root() / cargo_target_dir
endif

# --offline is required inside the Flatpak sandbox (crates are vendored).
cargo_flags = ['--locked']
if get_option('offline')
    cargo_flags += '--offline'
endif

cargo_common = [
    '--manifest-path', meson.project_source_root() / 'Cargo.toml',
    '--target-dir', cargo_target_dir,
    '--profile', cargo_profile,
]

# Always invoke cargo (it decides what is stale), then only replace the
# installed candidate if the bytes differ. Run from the source root so
# repository-local .cargo/config and rustup overrides apply.
build_script = '''
set -eu
cargo="$1"; bin="$2"; target="$3"; profile="$4"; output="$5"; source="$6"; shift 6
cd "$source"
"$cargo" build --bin "$bin" --target-dir "$target" --profile "$profile" "$@"
if [ "$profile" = dev ]; then dir=debug; else dir=release; fi
cmp -s "$target/$dir/$bin" "$output" || cp -f "$target/$dir/$bin" "$output"
'''

cargo_build = custom_target(
    'cargo-build',
    build_by_default: true,
    build_always_stale: true,
    output: bin_name,
    console: true,
    install: true,
    install_dir: bindir,
    install_mode: 'rwxr-xr-x',
    install_tag: 'runtime',
    depends: [resources, gsettings],
    command: [
        shell, '-c', build_script, 'cargo-build',
        cargo, bin_name, cargo_target_dir, cargo_profile,
        meson.current_build_dir() / bin_name, meson.project_source_root(),
        '--manifest-path', meson.project_source_root() / 'Cargo.toml',
    ] + cargo_flags,
)

if get_option('tests')
    test(
        'cargo-unit', cargo,
        args: ['test', '--workspace'] + cargo_flags + cargo_common,
        workdir: meson.project_source_root(),
        depends: cargo_build,
        suite: 'unit',
        timeout: get_option('test_timeout'),
        is_parallel: false,
    )

    if get_option('gui_tests')
        # One process per GTK test, and only these ignored tests ever run.
        gui_tests = [
            'window::tests::gtk_launcher_interactions_and_layout',
            'widgets::dropdown::tests::gtk_dropdown_template_selects_navigates_and_dismisses',
            'widgets::settings::tests::gtk_settings_template_persists_syncs_and_restores',
            'widgets::progressbar::tests::template_renders_progress_and_resets_safely',
        ]
        foreach gui_test : gui_tests
            test(
                gui_test.replace('::', '-'), cargo,
                args: ['test', '--workspace'] + cargo_flags + cargo_common + [
                    '--bin', bin_name, gui_test,
                    '--', '--exact', '--ignored', '--test-threads=1',
                ],
                workdir: meson.project_source_root(),
                depends: cargo_build,
                suite: 'gui',
                env: {'G_DEBUG': 'fatal-warnings'},
                timeout: get_option('test_timeout'),
                is_parallel: false,
            )
        endforeach
    endif
endif

summary({
    'Cargo profile': cargo_profile,
    'Cargo target directory': cargo_target_dir,
    'Offline': get_option('offline'),
    'Cargo unit tests': get_option('tests'),
    'Display-dependent GTK tests': get_option('gui_tests'),
}, section: 'Cargo integration', bool_yn: true)
