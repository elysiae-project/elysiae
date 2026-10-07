# Based on the instructions on the gtk4 rust cookbook
project('Elysiae', 'rust', version: '1.0.0', meson_version: '>= 1.4')

gnome = import('gnome')
base_id = 'app.elysiae.Elysiae'

is_devel = get_option('profile') == 'dev'
if is_devel
    profile = 'Devel'
    application_id = '@0@.@1@'.format(base_id, profile)
else
    profile = ''
    application_id = base_id
endif

bindir = get_option('prefix') / get_option('bindir')
localedir = get_option('prefix') / get_option('localedir')
datadir = get_option('prefix') / get_option('datadir')
pkgdatadir = datadir / meson.project_name()

subdir('data')
subdir('src')

gnome.post_install(
    gtk_update_icon_cache: true,
    glib_compile_schemas: true,
    update_desktop_database: true,
)
