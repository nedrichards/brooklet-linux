desktop_conf = configuration_data()
desktop_conf.set('APP_ID', app_id)

desktop_file = configure_file(
  input: 'com.nedrichards.brooklet.desktop.in',
  output: app_id + '.desktop',
  configuration: desktop_conf,
  install: true,
  install_dir: get_option('datadir') / 'applications',
)

metainfo_file = configure_file(
  input: 'com.nedrichards.brooklet.metainfo.xml.in',
  output: app_id + '.metainfo.xml',
  configuration: desktop_conf,
  install: true,
  install_dir: get_option('datadir') / 'metainfo',
)

install_data(
  'icons/scalable/apps/com.nedrichards.brooklet.svg',
  install_dir: get_option('datadir') / 'icons/hicolor/scalable/apps',
  rename: app_id + '.svg',
)

install_data(
  'icons/symbolic/apps/com.nedrichards.brooklet-symbolic.svg',
  install_dir: get_option('datadir') / 'icons/hicolor/symbolic/apps',
  rename: app_id + '-symbolic.svg',
)

test('desktop-file-validate', find_program('desktop-file-validate'), args: [desktop_file])
test('appstream-validate', find_program('appstreamcli'), args: ['validate', '--no-net', metainfo_file])
