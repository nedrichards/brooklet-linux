# Repository Guidelines

Use the pinned GNOME SDK and Rust extension in the Flatpak manifests.
See `docs/maintenance.md` for local builds, validation and releases.

## Local development and tests

Use `./scripts/meson.sh compile` and `./scripts/meson.sh test` for source
validation in the pinned SDK. Use `_build/` as the single local Meson build
directory and `_build/target/` as the shared Cargo target for builds, tests,
Clippy and specialised examples. Keep this reusable compilation cache. Do not
create a root `target/` or additional per-validation target trees. Do not run
Meson and GNOME Builder builds concurrently in this checkout.

## Local Flatpak build storage

Use `./scripts/local-flatpak.sh` for production builds and updates, or
`./scripts/local-flatpak.sh development` for the separate `.Devel` app.
Use the normal manifests and keep their test gates enabled.

- Keep the durable update repository at `flatpak-repo/`. Never place it under
  `.flatpak-builder/` or another disposable build/cache directory.
- Use `.flatpak-builder/` for Builder state and `.flatpak-builder/staging` as
  the single staging location for both profiles. Build sequentially. Do not
  create per-task, per-validation or timestamped build/state/repository trees.
- Use `--delete-build-dirs` and clean module working directories and staging
  after success, failure and ordinary interruption, as the script does. Keep
  the reusable cache and downloads. Do not use `--keep-build-dirs` for routine
  automated builds. Debug retention needs an explicit task-specific reason
  and cleanup when finished.
- Do not run GNOME Builder or raw Builder commands concurrently with the
  script. Interactive GNOME Builder development remains supported via the
  `.Devel` manifest; prefer the script for automated validation and deployment.
- Keep directory-source `skip` lists excluding generated build directories,
  `target`, `.flatpak-builder*` and `flatpak-repo`, so artifacts are not copied
  into subsequent builds.
- Verify the installed app and its update origin after deployment. Never
  re-export an installed `export/` directory.

There is no enforced storage budget. Do not change Rust incremental/debug
settings as part of storage housekeeping without a separate request.
