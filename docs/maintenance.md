# Maintenance and release checks

Dependency updates are proposed weekly by Dependabot for GitHub Actions and applicable language manifests. The Dependency health workflow runs weekly and on demand: it reports GNOME runtime/SDK and Rust SDK extension end-of-life, unavailable runtime refs, and outdated bundled sources via Flatpak External Data Checker. Pull requests validate dependency-checker coverage without blocking source changes on already-known update reports. Reports appear in the Actions summary and remain downloadable for 14 days. A network/checker failure is a failed check, not evidence that dependencies are current. Rust dependency advisories are checked separately by RustSec. Dependency reports suggest changes; they never repin a stable application source or publish releases.

The required source and Flatpak jobs retain the repository's application-specific tests. Development artifacts have a separate .Devel identity, a checkout SHA, a 14-day lifetime, and an installation command in the summary. SDK/build success does not establish hardware, live-service, portal, compositor, or multi-monitor behaviour; keep the existing manual and installed-app gates in the release process.

## Local Flatpak installation

From the repository root, build and test the production manifest into the local
Flatpak repository:

```sh
flatpak-builder --user --disable-rofiles-fuse --install-deps-from=flathub --force-clean \
  --default-branch=master \
  --repo="$PWD/.flatpak-builder/brooklet-release-repo" \
  build/release flatpak/com.nedrichards.brooklet.json
```

This runs the manifest's tests and lets Flatpak Builder handle installation into
the build directory, cleanup, finishing and export. `--force-clean` replaces
only the build directory; Builder still reuses its build cache. Disabling
rofiles-fuse uses Flatpak Builder's copy based build mode, needed on hosts where
FUSE mounts are unavailable. The manifest selects the production application
ID automatically.

After the build and tests pass, register the local repository and install or
update its package:

```sh
flatpak remote-add --user --if-not-exists --no-gpg-verify \
  brooklet-local "$PWD/.flatpak-builder/brooklet-release-repo"
flatpak install --user --or-update brooklet-local com.nedrichards.brooklet//master
```

The existing `brooklet-local` remote points to this repository. Subsequent
deployments use the same commands. Quit and reopen Brooklet from GNOME Shell
after installation to use the new build; its account and cached data are kept.

For a separate development installation:

```sh
flatpak-builder --user --install --install-deps-from=flathub --force-clean \
  build-dir flatpak/com.nedrichards.brooklet.Devel.json
```

Its `.Devel` application ID has a separate profile and does not update the
production installation.

Never reuse an installed `export/` directory for `flatpak build-export`: its
desktop entry already contains the Flatpak wrapper. Exporting it again nests
`flatpak run` and makes GNOME Shell try to run Flatpak inside the app sandbox.

## Releases

The production profile is `flatpak/com.nedrichards.brooklet.json`. Dispatch `Publish Release Flatpaks` from the default branch with an existing `vX.Y.Z` tag. Its tagged production manifest is converted to an exact Git commit in a temporary build manifest. Both x86_64 and aarch64 builds, SDK tests and the physical-keyboard regression must pass before publication, with SHA256SUMS. Existing releases are never overwritten. The first release still requires the documented live-server and visual/accessibility checks.

GitHub bundles are sideloadable packages. Flathub updates continue through the Flathub repository review process. This workflow does not submit to or publish on Flathub.

Flatpak Meson tests run serially with a timeout multiplier of three. Cargo's shared target lock otherwise makes concurrent format, Clippy and test jobs time out while a release build warms the cache.

GitHub repository settings also enable dependency vulnerability alerts, Dependabot security updates, and weekly CodeQL default setup. Security updates propose pull requests; they do not merge them or publish app releases. The default CodeQL configuration lives in GitHub settings, alongside these versioned maintenance workflows.

## Hosted desktop observations

Full `--smoke-test` journeys remain available for installed-app local validation. Hosted Xvfb runs can fail different geometry, viewport or decoder assertions without a stable desktop session. CI and release workflows therefore run this suite as an advisory step with an eight-minute bound, a 14-day report artifact, and an explicit outcome in the summary. No assertion is removed or retried until green. Rust tests, format, Clippy, SDK Meson checks, metadata/build/export and the physical-keyboard gate stay blocking. A hosted green build does not claim a passing full desktop smoke suite; inspect its separate outcome. Before releasing, continue the existing local installed-app validation.
