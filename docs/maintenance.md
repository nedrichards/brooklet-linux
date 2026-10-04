# Maintenance and release checks

Dependency updates are proposed weekly by Dependabot for GitHub Actions and applicable language manifests. The Dependency health workflow runs weekly and on demand: it reports GNOME runtime/SDK and Rust SDK extension end-of-life, unavailable runtime refs, and outdated bundled sources via Flatpak External Data Checker. Pull requests validate dependency-checker coverage without blocking source changes on already-known update reports. Reports appear in the Actions summary and remain downloadable for 14 days. A network/checker failure is a failed check, not evidence that dependencies are current. Rust dependency advisories are checked separately by RustSec. Dependency reports suggest changes; they never repin a stable application source or publish releases.

The required source and Flatpak jobs retain the repository's application-specific tests. Development artifacts have a separate .Devel identity, a checkout SHA, a 14-day lifetime, and an installation command in the summary. SDK/build success does not establish hardware, live-service, portal, compositor, or multi-monitor behaviour; keep the existing manual and installed-app gates in the release process.

## Releases

The production profile is `flatpak/com.nedrichards.brooklet.json`. Dispatch `Publish Release Flatpaks` from the default branch with an existing `vX.Y.Z` tag. Its tagged production manifest is converted to an exact Git commit in a temporary build manifest. Both x86_64 and aarch64 builds and the required GTK/keyboard regressions must pass before publication, with SHA256SUMS. Existing releases are never overwritten. The first release still requires the documented live-server and visual/accessibility checks.

GitHub bundles are sideloadable packages. Flathub updates continue through the Flathub repository review process. This workflow does not submit to or publish on Flathub.

Flatpak Meson tests run serially with a timeout multiplier of three. Cargo's shared target lock otherwise makes concurrent format, Clippy and test jobs time out while a release build warms the cache.

GitHub repository settings also enable dependency vulnerability alerts, Dependabot security updates, and weekly CodeQL default setup. Security updates propose pull requests; they do not merge them or publish app releases. The default CodeQL configuration lives in GitHub settings, alongside these versioned maintenance workflows.
