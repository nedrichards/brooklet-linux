# Brooklet

Brooklet is an opinionated, offline-first Miniflux client for GNOME. It helps
you get through what is new, keep what matters, and read comfortably in a
native GTK interface.

Inbox, Saved and Library share the same controls. Double-click or double-tap
the active destination to return its current list to the top without moving
the reader.

There is also a separately developed Android application with the same vibe,
but a totally different implementation. I hope you enjoy either, neither or
both.

![Brooklet showing the Inbox and article reader](data/screenshots/inbox-screenshot.png)

## Install a release

Download the Flatpak bundle for your architecture from
[GitHub Releases](https://github.com/nedrichards/brooklet-linux/releases).

```sh
flatpak install --user ./Brooklet-v0.1.2-x86_64.flatpak
```

Use the `aarch64` bundle for ARM64 systems. GitHub bundles are release snapshots;
install a newer bundle to update. For the updateable local `brooklet-local`
repository, see [Local Flatpak installation](docs/maintenance.md#local-flatpak-installation).

## Development and testing

Use Meson inside the pinned GNOME SDK. The helper configures `_build/` on
first use and retains its Rust compilation cache for subsequent builds:

```sh
./scripts/meson.sh compile
./scripts/meson.sh test
```

Run a particular check with, for example:

```sh
./scripts/meson.sh test rust-unit-tests
```

Meson runs core tests, reader lifecycle tests, formatting, Clippy and metadata
validation. Compilation and checks share `_build/target/`; avoid standalone
Cargo commands that create a second root `target/` tree. See
[maintenance](docs/maintenance.md#development-builds-and-tests) for details.

Setup logic is tested without credentials or a live service through injected
validator, repository, and secret-store implementations. The Flatpak smoke
test also constructs the main window, setup dialog, and shortcuts dialog:

```sh
./scripts/local-flatpak.sh development
flatpak run --user com.nedrichards.brooklet.Devel --smoke-test
```

## Licence

Brooklet is licensed under GPL-3.0-or-later. See `COPYING`.
