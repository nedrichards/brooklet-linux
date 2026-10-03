project(
  'brooklet',
  version: '0.1.0',
  meson_version: '>= 1.4.0',
  license: 'GPL-3.0-or-later',
)

gnome = import('gnome')
i18n = import('i18n')

app_id = 'com.nedrichards.brooklet'
profile = get_option('profile')
if profile == 'development'
  app_id = app_id + '.Devel'
endif

subdir('data')
subdir('po')

cargo = find_program('cargo', '/usr/lib/sdk/rust-stable/bin/cargo')
cargo_build = find_program('build-aux/cargo-build.sh')

cargo_target = custom_target(
  'brooklet-binary',
  input: files(
    'Cargo.toml',
    'Cargo.lock',
    'build.rs',
    'src/main.rs',
    'src/application.rs',
    'src/keyboard.rs',
    'src/auto_refresh.rs',
    'src/outgoing.rs',
    'src/config.rs',
    'src/controller.rs',
    'src/error.rs',
    'src/lib.rs',
    'src/logging.rs',
    'src/model/mod.rs',
    'src/api/mod.rs',
    'src/api/miniflux.rs',
    'src/api/karakeep.rs',
    'src/reader/mod.rs',
    'src/reader/parser.rs',
    'src/services/mod.rs',
    'src/services/image_cache.rs',
    'src/services/secret_store.rs',
    'src/services/traits.rs',
    'src/services/url_policy.rs',
    'src/setup.rs',
    'src/storage/mod.rs',
    'src/storage/sqlite.rs',
    'src/ui/mod.rs',
    'src/ui/entry_object.rs',
    'src/ui/inbox.rs',
    'src/ui/reader.rs',
    'src/ui/reader_table.rs',
    'src/ui/reader_images.rs',
    'src/ui/setup.rs',
    'src/ui/reconnect.rs',
    'src/ui/karakeep.rs',
    'src/ui/library.rs',
    'src/ui/library_smoke.rs',
    'src/ui/signal_scope.rs',
    'src/sync/mod.rs',
    'resources/ui/window.blp',
    'resources/ui/setup-dialog.blp',
    'resources/ui/shortcuts-dialog.blp',
    'resources/style.css',
  ),
  output: 'brooklet',
  command: [
    cargo_build,
    meson.project_source_root(),
    meson.project_build_root(),
    '@OUTPUT@',
    profile,
    app_id,
    cargo.full_path(),
  ],
  build_by_default: true,
  install: true,
  install_dir: get_option('bindir'),
)

test(
  'rust-unit-tests',
  cargo,
  args: ['test', '--all-targets', '--locked'],
  workdir: meson.project_source_root(),
  timeout: 180,
)

test(
  'rust-format',
  cargo,
  args: ['fmt', '--all', '--check'],
  workdir: meson.project_source_root(),
)

test(
  'rust-clippy',
  cargo,
  args: ['clippy', '--all-targets', '--locked', '--features', 'gui', '--', '-D', 'warnings'],
  workdir: meson.project_source_root(),
  timeout: 300,
)

summary({
  'Application ID': app_id,
  'Profile': profile,
  'GNOME target': '51 (GTK 4.24, libadwaita 1.10)',
}, section: 'Brooklet')
