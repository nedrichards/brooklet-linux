# Maintenance and release checks

Dependency updates are proposed weekly by Dependabot for GitHub Actions and applicable language manifests. The Dependency health workflow runs weekly and on demand: it reports GNOME runtime/SDK and Rust SDK extension end-of-life, unavailable runtime refs, and outdated bundled sources via Flatpak External Data Checker. Pull requests validate dependency-checker coverage without blocking source changes on already-known update reports. Reports appear in the Actions summary and remain downloadable for 14 days. A network/checker failure is a failed check, not evidence that dependencies are current. Rust dependency advisories are checked separately by RustSec. Dependency reports suggest changes; they never repin a stable application source or publish releases.

The required source and Flatpak jobs retain the repository's application-specific tests. Development artifacts have a separate .Devel identity, a checkout SHA, a 14-day lifetime, and an installation command in the summary. SDK/build success does not establish hardware, live-service, portal, compositor, or multi-monitor behaviour; keep the existing manual and installed-app gates in the release process.

## Development builds and tests

Use `./scripts/meson.sh compile` and `./scripts/meson.sh test` for local
source validation. The helper runs Meson inside the SDK named in the development
manifest, with its Rust extension on PATH. Install that SDK and extension first.
It enables network access for missing Cargo dependencies and keeps downloaded
crates in `_build/cargo-home/`.

The sole local Meson build directory is `_build/`. Build and test commands use
`_build/target/` for Cargo output and the same application identity, so checks
reuse compiled dependencies. Keep this directory between development sessions.
Meson serializes Cargo checks to avoid competing for the shared target lock.
Use `./scripts/meson.sh test rust-unit-tests` for a focused core check, or
`./scripts/meson.sh test --list` to list all checks.

Do not run standalone Cargo builds/tests into root `target/`, or create extra
SDK/validation target directories. For a specialised Cargo example that Meson
does not expose, run it inside the pinned SDK with
`CARGO_TARGET_DIR="$PWD/_build/target"`; reuse `_build/cargo-home/` for downloads.
Do not run this helper while GNOME Builder is building the same checkout.

This development cache is separate from production Flatpak packaging. The
canonical Flatpak script below still builds and tests the normal manifest in
its disposable module directory. Removing root `target/` does not affect the
installed apps. Removing `_build/target/` forces the next development compile
and check to rebuild dependencies; it is not routine cleanup. Keep logs and
review artifacts under `_build/` until their purpose has been reviewed.

## Local Flatpak installation

Run the canonical build from the repository root:

```sh
./scripts/local-flatpak.sh
```

This builds and tests the production manifest, exports to `flatpak-repo/`,
then installs or updates the user application. Quit and reopen the app after
updating. Account settings and application data are preserved.

For the separate development application, use:

```sh
./scripts/local-flatpak.sh development
```

Both profiles share one staging directory, `.flatpak-builder/staging`, and
one Builder state directory, `.flatpak-builder/`. The script takes a checkout
lock, uses `--force-clean --delete-build-dirs`, and removes module working
directories and staging on success, failure, or an ordinary interrupt. SIGKILL
or power loss can leave working files; the next run reuses the same paths and
cleans them on exit. Do not run GNOME Builder or another build in this checkout
at the same time. Downloads and Builder's reusable cache remain available;
this is not a storage quota or a limit on cache growth.

The build log is `.flatpak-builder/local-build.log` and is replaced on each
run. Run diagnostic commands and installed-app checks against `flatpak run`
after installation; staging is deliberately disposable. GNOME Builder can
still use the `.Devel` manifest for interactive development. Prefer the script
for automated builds and validation; do not create extra validation trees.

`flatpak-repo/` is the durable local update repository, outside disposable
build state. Keep it when cleaning `.flatpak-builder/`. The production remote
is `brooklet-local`; development uses `brooklet-devel-local`. The script registers or repoints
these remotes only after the build, tests and export succeed. Source manifests
exclude the repository and generated build directories from directory copies.

Never export an installed `export/` directory with `flatpak build-export`:
its desktop entry already includes the host Flatpak wrapper. Let the normal
manifest-driven build finish and export the staging tree.

## Releases

The production profile is `flatpak/com.nedrichards.brooklet.json`. Dispatch `Publish Release Flatpaks` from the default branch with an existing `vX.Y.Z` tag. Its tagged production manifest is converted to an exact Git commit in a temporary build manifest. Both x86_64 and aarch64 builds, SDK tests and the physical-keyboard regression must pass before publication, with SHA256SUMS. Existing releases are never overwritten. The first release still requires the documented live-server and visual/accessibility checks.

Update `Cargo.toml`, the Brooklet package entry in `Cargo.lock`, `meson.build`,
and the AppStream release history together before tagging. Commit the release,
then push `main` and its annotated tag. Start the release workflow with:

```sh
gh workflow run release-flatpak.yml --repo nedrichards/brooklet-linux \
  --ref main -f tag=vX.Y.Z
```

The workflow builds the tagged source, attaches both bundles and `SHA256SUMS`
to a draft GitHub release, and publishes it only after the required checks pass.
Workflow-only fixes can be committed to `main` and dispatched for the same tag
without moving the tag or changing its application source. A failed run creates
no published release; inspect its logs before dispatching again.

GitHub bundles are sideloadable packages. Flathub updates continue through the Flathub repository review process. This workflow does not submit to or publish on Flathub.

Flatpak Meson tests run serially with a timeout multiplier of three. Cargo's shared target lock otherwise makes concurrent format, Clippy and test jobs time out while a release build warms the cache.

GitHub repository settings also enable dependency vulnerability alerts, Dependabot security updates, and weekly CodeQL default setup. Security updates propose pull requests; they do not merge them or publish app releases. The default CodeQL configuration lives in GitHub settings, alongside these versioned maintenance workflows.

## Hosted desktop observations

Full `--smoke-test` journeys remain available for installed-app local validation. Hosted Xvfb runs can fail different geometry, viewport or decoder assertions without a stable desktop session. CI and release workflows therefore run this suite as an advisory step with an eight-minute bound, a 14-day report artifact, and an explicit outcome in the summary. No assertion is removed or retried until green. Rust tests, format, Clippy, SDK Meson checks, metadata/build/export and the physical-keyboard gate stay blocking. A hosted green build does not claim a passing full desktop smoke suite; inspect its separate outcome. Before releasing, continue the existing local installed-app validation.
