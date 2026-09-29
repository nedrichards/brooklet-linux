# Reader testing

Rendering tests use article HTML and metadata, not Miniflux credentials.
The app stores tokens in the system keyring via oo7. Core sync tests inject
fake services; use a separate Miniflux test account for live mutation tests.
Do not copy production tokens into fixtures or environment files.

## Private cache review

Export a consistent read-only snapshot of Read and Saved articles:

```sh
python3 build-aux/export-reader-corpus.py \
  "$HOME/.var/app/com.nedrichards.brooklet.Devel/data/com.nedrichards.brooklet.Devel/brooklet.db" \
  _build/render-audit/articles.json
```

The export contains article records only. Article text and URLs can still be
personal; keep exports, parsed documents, and screenshots under ignored
`_build/`. Only small synthetic HTML examples belong in the checked-in tests.

Build the preview with the GNOME SDK:

```sh
cargo build --features gui --example reader_review
```

Run it in the app sandbox (using the SDK target path for local SDK builds):

```sh
flatpak build --socket=wayland --socket=x11 --env=GSK_RENDERER=cairo \
  --talk-name=org.freedesktop.portal.Flatpak \
  --filesystem="$PWD" build-dir \
  "$PWD/_build/sdk-target/debug/examples/reader_review" \
  "$PWD/_build/render-audit/articles.json" \
  "$PWD/_build/render-audit/review" 37963
```

Every corpus article is parsed into `blocks.json`. Specified article IDs get
native GTK screenshots at 400 and 760 pixels, at the top, middle, bottom, and each table.
No database, keyring, read state, stars, or reader positions are accessed by
the preview. Images stay as placeholders by default.

Add `--share=network` to `flatpak build` and `--images` before the article IDs
to fetch and decode images through the app's public-image policy and Glycin.
Failures retain a visible placeholder and are reported. A direct SDK-runtime
launch can render text, but its missing Application metadata prevents the
Flatpak portal from launching Glycin; use the app environment for image
verification. Glycin disables its additional sandbox under `flatpak build`,
so this preview does not verify decoder isolation in an installed Flatpak.

The screenshots sample the viewport, not every pixel of a long article.
Inspect the parsed documents and select additional cases when reviewing
specific structures. Merged table cells use native grid layout; wide tables scroll horizontally.
Interactive media has external links instead.

## Interactive development app

After building and updating `_build/brooklet`, run:

```sh
build-aux/run-dev.sh
```

This launcher explicitly uses the installed development app's data, config,
and cache directories, including its existing keyring storage. A bare
`flatpak build` launch can use an empty, separate profile and show setup.
This interactive launcher uses your real account and normal sync; use the
isolated reader preview above for tests that must not touch account state.
Close an existing Brooklet instance before launching a different binary.

## Image cache policy

The app stores image bytes in `$XDG_CACHE_HOME/com.nedrichards.brooklet.Devel/images.db`.
It holds up to 256 MiB of image payloads and 8,192 entries, with least recently
used eviction. SQLite metadata and journal files add storage overhead. Cache
hits do not make network requests. Images are saved as articles are opened;
this does not pre-download every synced article or pin all Saved images.
The same URL is retained until eviction or an explicit clear.

SQLite work runs on blocking backend threads. Downloads share one HTTP client
and are limited to four, decoding to two. Switching articles cancels pending
downloads, and generation checks reject stale decode results. Failed decodes
are evicted. Preferences offers Clear under Article images; logout also clears
the image cache. The cache is disposable and never holds service credentials.

The preview uses its own `OUTPUT_DIR/images.db`, isolated from the app cache.
Populate it with `--images`, then run a new preview process with
`--offline-images` and the same output directory. Offline mode refuses fetches.
For additional verification, run the latter with `flatpak build --unshare=network`.
