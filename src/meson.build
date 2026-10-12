cargo = find_program('cargo')
shell = find_program('sh')
rust_target = is_devel ? 'debug' : 'release'
cargo_command = 'target="${CARGO_TARGET_DIR:-$3}"; "$1" build --locked --manifest-path "$2/Cargo.toml" --target-dir "$target"'
if not is_devel
    cargo_command += ' --release'
endif
cargo_command += '; cp -- "$target/$4/Elysiae" "$5"'

custom_target(
    'cargo-build',
    build_by_default: true,
    build_always_stale: true,
    output: meson.project_name(),
    console: true,
    install: true,
    install_dir: bindir,
    depends: [resources, gsettings],
    command: [
        shell, '-eu', '-c', cargo_command, 'cargo-build',
        cargo.full_path(),
        meson.project_source_root(),
        meson.project_build_root() / 'target',
        rust_target,
        '@OUTPUT@',
    ],
)
