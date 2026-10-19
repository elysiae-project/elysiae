
subdir('resources')
subdir('icons')
subdir('fonts')
desktop_file = configure_file(
    input: 'app.elysiae.Elysiae.desktop.in',
    output: '@0@.desktop'.format(application_id),
    configuration: {
        'APP_ID': application_id,
    },
)
install_data(desktop_file, install_dir: datadir / 'applications', install_tag: 'runtime')

metainfo_file = configure_file(
    input: 'app.elysiae.Elysiae.metainfo.xml.in',
    output: '@0@.metainfo.xml'.format(application_id),
    configuration: {
        'APP_ID': application_id,
    },
)
install_data(metainfo_file, install_dir: datadir / 'metainfo', install_tag: 'runtime')

install_data(
    'app.elysiae.Elysiae.gschema.xml',
    install_dir: datadir / 'glib-2.0' / 'schemas',
    install_tag: 'runtime',
)

if get_option('tests')
    test(
        'gsettings-schema', schema_compiler,
        args: ['--strict', '--dry-run', meson.current_source_dir()],
        suite: 'packaging',
    )
    desktop_validator = find_program('desktop-file-validate', required: get_option('validation'))
    appstream_validator = find_program('appstreamcli', required: get_option('validation'))
    if desktop_validator.found()
        test(
            'desktop-entry', desktop_validator,
            args: [desktop_file],
            suite: 'packaging',
        )
    elif get_option('validation').auto()
        warning('desktop-file-validate not found: desktop-entry test omitted. Use -Dvalidation=enabled to require it.')
    endif
    if appstream_validator.found()
        test(
            'appstream-metadata', appstream_validator,
            args: ['validate', '--no-net', metainfo_file],
            suite: 'packaging',
        )
    elif get_option('validation').auto()
        warning('appstreamcli not found: AppStream test omitted. Use -Dvalidation=enabled to require it.')
    endif
    summary({
        'Strict GSettings schema validation': true,
        'Desktop entry validation': desktop_validator.found(),
        'Offline AppStream validation': appstream_validator.found(),
    }, section: 'Packaging tests', bool_yn: true)
endif

subdir('po')
