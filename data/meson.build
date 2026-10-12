
subdir('resources')
subdir('icons')
subdir('fonts')
desktop_file = configure_file(
    input: 'app.elysiae.Elysiae.desktop.in',
    output: '@0@.desktop'.format(application_id),
    configuration: {
        'APP_ID': application_id,
        'PROFILE': profile,
    },
)
install_data(desktop_file, install_dir: datadir / 'applications')

metainfo_file = configure_file(
    input: 'app.elysiae.Elysiae.metainfo.xml.in',
    output: '@0@.metainfo.xml'.format(application_id),
    configuration: {
        'APP_ID': application_id,
    },
)
install_data(metainfo_file, install_dir: datadir / 'metainfo')

install_data(
    'app.elysiae.Elysiae.gschema.xml',
    install_dir: datadir / 'glib-2.0' / 'schemas',
)

subdir('po')
