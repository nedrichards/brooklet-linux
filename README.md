# Brooklet

Brooklet is an opinionated, offline-first Miniflux client for GNOME. It helps
you get through what is new, keep what matters, and read comfortably in a
native GTK interface.

There is also a separately developed Android application with the same vibe,
but a totally different implementation. I hope you enjoy either, neither or
both.

## Development status

The GNOME 51 application now includes setup, incremental offline-first sync,
Inbox, Saved, Library, native reading, local search, and optional Karakeep
delivery. The repository and sync contracts have automated coverage; the next
release gate is live-server use and visual/accessibility refinement.

The canonical build is the Flatpak manifest at
`flatpak/com.nedrichards.brooklet.Devel.json`. It targets GNOME 51 and is placed
where GNOME Builder discovers it from a fresh checkout.

## Native build

```sh
meson setup build
meson compile -C build
meson test -C build
```

Native development requires GTK 4.24, libadwaita 1.10, Blueprint Compiler,
Meson, Cargo, and Rust 1.93 or later but Flatpak is the only supported or
encouraged environment.

## Core tests without GTK

The non-UI domain and service code deliberately builds without GTK:

```sh
cargo test --all-targets
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Setup logic is tested without credentials or a live service through injected
validator, repository, and secret-store implementations. The Flatpak smoke
test also constructs the main window, setup dialog, and shortcuts dialog:

```sh
flatpak-builder --run build-dir \
  flatpak/com.nedrichards.brooklet.Devel.json \
  brooklet --smoke-test
```

## Licence

Brooklet is licensed under GPL-3.0-or-later. See `COPYING`.
