fs = import('fs')

# Capture the cache at configure time so build and test use exactly the same
# directory, even when their environments differ. The option survives reconfigure;
# environment-only selections must be supplied again when reconfiguring.
cargo_target_dir = get_option('cargo_target_dir')
if cargo_target_dir == ''
    cargo_target_dir = run_command(
        shell, '-c', 'printf "%s" "${CARGO_TARGET_DIR:-}"',
        check: true,
    ).stdout()
endif
if cargo_target_dir == ''
    cargo_target_dir = meson.project_build_root() / 'target'
elif not fs.is_absolute(cargo_target_dir)
    cargo_target_dir = meson.project_source_root() / cargo_target_dir
endif

# Cargo owns Rust dependency tracking and profile settings. Always invoke it,
# but avoid touching the installed-output candidate when its bytes are unchanged.
# Start in the source root for rustup and repository-local .cargo configuration.
cargo_preamble = '''
cargo="$1"; source="$2"; target="$3"; profile="$4"; shift 4
cd "$source"
if [ -n "${CARGO_BUILD_TARGET:-}" ]; then
    echo 'CARGO_BUILD_TARGET is not supported by this native Meson build; unset it.' >&2
    exit 1
fi
'''
cargo_args = [
    cargo.full_path(),
    meson.project_source_root(),
    cargo_target_dir,
    cargo_profile,
]
# Read Cargo's artifact report instead of assuming an old executable in the
# cache belongs to this build. Keep this bridge here, not in a helper script.
cargo_build_script = '''
import filecmp, json, os, pathlib, shutil, subprocess, sys, tempfile
cargo, source, target, profile, output = sys.argv[1:]
if os.environ.get('CARGO_BUILD_TARGET'):
    sys.exit('CARGO_BUILD_TARGET is not supported by this native Meson build; unset it.')
expected = pathlib.Path(target, 'debug' if profile == 'dev' else 'release', 'Elysiae').resolve()
executable = None
command = [cargo, 'build', '--locked', '--manifest-path', source + '/Cargo.toml',
           '--target-dir', target, '--profile', profile, '--bin', 'Elysiae',
           '--message-format=json-render-diagnostics']
with subprocess.Popen(command, cwd=source, stdout=subprocess.PIPE, text=True) as process:
    for line in process.stdout:
        message = json.loads(line)
        if (message.get('reason') == 'compiler-artifact'
                and message.get('target', {}).get('name') == 'Elysiae'
                and 'bin' in message.get('target', {}).get('kind', [])
                and message.get('executable')
                and not message.get('profile', {}).get('test')):
            executable = pathlib.Path(message['executable']).resolve()
    status = process.wait()
if status:
    sys.exit(status)
if executable != expected:
    sys.exit('Cargo did not report the expected native executable: ' + str(expected)
             + '. Cargo build.target overrides are not supported.')
if os.path.isfile(output) and filecmp.cmp(executable, output, shallow=False):
    shutil.copymode(executable, output)
else:
    fd, temporary = tempfile.mkstemp(dir=os.path.dirname(output), prefix='Elysiae.')
    os.close(fd)
    try:
        shutil.copyfile(executable, temporary)
        shutil.copymode(executable, temporary)
        os.replace(temporary, output)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)
'''

cargo_build = custom_target(
    'cargo-build',
    build_by_default: true,
    build_always_stale: true,
    output: meson.project_name(),
    console: true,
    install: true,
    install_dir: bindir,
    install_mode: 'rwxr-xr-x',
    install_tag: 'runtime',
    depends: [resources, gsettings],
    command: [
        python, '-c', cargo_build_script,
    ] + cargo_args + [
        meson.current_build_dir() / meson.project_name(),
    ],
)

if get_option('tests')
    cargo_test_command = cargo_preamble + '''
exec "$cargo" test --workspace --locked --manifest-path "$source/Cargo.toml" --target-dir "$target" --profile "$profile" "$@"
'''
    test(
        'cargo-unit', shell,
        args: ['-eu', '-c', cargo_test_command, 'cargo-test'] + cargo_args,
        workdir: meson.project_source_root(),
        depends: cargo_build,
        suite: 'unit',
        timeout: get_option('test_timeout'),
        is_parallel: false,
    )

    if get_option('gui_tests')
        # Serial Rust tests can still use different threads. Give each GTK test
        # its own process, and never run unrelated ignored tests accidentally.
        gui_check = '''
if [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
    echo 'GTK tests require a working display (DISPLAY or WAYLAND_DISPLAY). Disable with -Dgui_tests=false for headless testing.' >&2
    exit 1
fi
'''
        gui_tests = [
            'window::tests::gtk_launcher_interactions_and_layout',
            'widgets::dropdown::tests::gtk_dropdown_template_selects_navigates_and_dismisses',
            'widgets::settings::tests::gtk_settings_template_persists_syncs_and_restores',
            'widgets::progressbar::tests::template_renders_progress_and_resets_safely',
        ]
        foreach gui_test : gui_tests
            test(
                gui_test.replace('::', '-'), shell,
                args: ['-eu', '-c', gui_check + cargo_test_command, 'cargo-gui-test'] + cargo_args + [
                    '--bin', meson.project_name(), gui_test,
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
    'Cargo unit tests': get_option('tests'),
    'Display-dependent GTK tests': get_option('gui_tests'),
}, section: 'Cargo integration', bool_yn: true)
