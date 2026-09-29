# Performance and efficiency review — 29–30 September 2026

The app is not yet demonstrated to sustain 60 fps. At 60 Hz, a frame has
16.67 ms for application work, layout, and rendering together. This review
finds concrete avoidable work, fixes several hot paths, and separates
microbenchmarks from end-to-end frame and power measurements.

## Changes made

- Backend completion now wakes the GLib main context through a Tokio oneshot
  channel. The previous bridge woke every 25 ms per pending operation,
  including slow downloads, and added up to 25 ms of completion latency.
- List reconciliation now compares borrowed entries, retains unchanged
  objects, and splices one changed interval. Previously it searched through
  the remaining list for every moved/new row, cloning full article bodies
  during those searches. Unchanged refreshes emit no model changes.
- Previous/Next navigation snapshots share immutable article entries with
  the originating list. Opening an article previously copied the full source
  list, including every HTML body. State changes use copy-on-write so they
  cannot silently change the list's immutable objects.
- Scroll-position persistence cancels the previous debounce source. At most
  one save timer remains pending instead of a timer for every scroll event.

Existing reader, keyboard, Undo, and focus work was present before this pass
and is preserved. This initial checkpoint was published as `0f6a1ac` and
`9b23da9`. Private corpus exports and local review output were not published.

## Follow-up implementation — 30 September

- List queries now return metadata and a body revision rather than HTML. The
  account-scoped body is fetched only when activated. Library queries run for
  mapped views, with generation checks rejecting superseded responses.
- Parsing runs behind one backend permit. Switching articles aborts queued
  requests and rejects stale results. GTK construction runs in frame callbacks,
  with at most eight tasks or approximately 4 ms per callback; table cells are
  separate tasks. Position restoration waits for construction and layout.
- Image work starts within one viewport of the visible region, prioritizing
  the closest images. There are at most two fetch/decode jobs, with cancellation
  when they leave that region or the reader closes. Decode requests use display
  width, at most 2 Mi pixels and 4,096 pixels per side; sources above 32 Mi pixels
  are rejected. Returned frame dimensions and buffer lengths are checked too.
  Resident decoded payloads are capped at 64 MiB, evicting offscreen textures.
  Oversized or unavailable images retain their external-source placeholders.
- Reader/list callbacks capture weak widgets. Image adjustment callbacks are
  disconnected on teardown, and dialog callbacks are scoped to closure or
  destruction. Scroll-position geometry is coalesced to a 250 ms pause rather
  than traversed every frame. Switching and closing still capture the position.
- Repository operations acquire one asynchronous permit before dispatching
  synchronous SQLite work to a blocking thread. Superseded searches cancel
  queued work, while a transaction already running completes normally.

These budgets reduce avoidable frame work; they do not guarantee 60 fps. A
single large text block, GTK layout, or shaping can still exceed a frame budget.
The decoded payload cap is not a process-tree RSS or GPU-memory ceiling;
decoder helpers can allocate transiently. Database opening and migrations are
still synchronous at startup. Optimized frame, memory, and power profiling
remains necessary.

## Measurements

Measurements use the installed GNOME 51 SDK and its default debug build.
They are single-run diagnostics, not release budgets or frame-rate results.

The synthetic list probe uses 5,000 articles with 16 KiB bodies (78 MiB of
HTML per snapshot). Input creation is outside the timed region; reconciliation
and disposal of the incoming snapshot are inside it. It exercises a real
Gio ListStore and selection model, without a mapped ListView, so it excludes
row binding, GTK layout, and rendering.

| Operation | Original | Updated |
| --- | ---: | ---: |
| Unchanged snapshot | 15.49 ms | 15.84 ms |
| Prepend one article | 28.66 ms | 11.60 ms |
| Reverse the list | 21,575 ms | 17.92 ms |
| Remove alternate articles | 9,498 ms | 10.33 ms |

The original scan was quadratic for reordering. The update has linear
reconciliation work plus body comparisons. The unchanged case shows why
avoiding body-bearing incoming snapshots remains necessary: removing the
scan alone does not eliminate comparison and destruction costs.

A read-only aggregate audit of the installed development cache found:

| View | Articles | HTML bytes |
| --- | ---: | ---: |
| All | 2,724 | 16,759,673 |
| Unread | 178 | 2,668,478 |
| Read | 2,546 | 14,091,195 |
| Saved | 14 | 51,245 |

No credentials, titles, article URLs, or private content are recorded here.
The read-only export and baseline source copies remain under ignored `_build/`.

Parsing the 2,546 Read/Saved corpus articles produced 52,151 document blocks:
p50 0.960 ms, p95 3.607 ms, p99 5.754 ms, maximum 17.909 ms. Two parses alone
exceeded 16.67 ms. These numbers exclude GTK widget creation, text shaping,
layout, image decoding, and drawing, so they cannot demonstrate smooth opening
or scrolling. No live account mutations or network fetches were used.

## Findings at the initial checkpoint

The following findings motivated the follow-up above. They describe the
29 September code, rather than the current implementation.

1. **Article opening can stall the UI.** `ui::reader::show` parses HTML and
   creates all block widgets synchronously. Large articles and tables can
   exceed the frame budget even before layout. Move parsing to bounded backend
   work, reject stale results by generation, and construct long documents in
   bounded batches. Measure text shaping/layout separately; background parsing
   alone does not fix eager widget creation.
2. **Views duplicate complete bodies and reload hidden lists.**
   `load_other_views` queries Saved, All, Unread, and Read after sync and star
   changes. Each query materializes full HTML. Together with Inbox this cache
   represents about 34.6 MiB of duplicated HTML before object/string overhead,
   pending snapshots, or textures. Use lightweight list summaries and fetch
   the body on activation; load hidden views on demand and invalidate them
   without eagerly rebuilding. Preserve Previous/Next and offline semantics.
3. **Image memory and work are not bounded by the disk cache.** The reader
   fetches every image immediately, including offscreen images. Four download
   slots and two decode slots bound concurrency, but queued decode futures
   retain downloaded buffers and every displayed texture stays resident.
   The 20 MiB compressed-input limit does not bound decoded pixel memory.
   Switching articles aborts fetches and rejects stale decode results, but
   already-running decode futures have no equivalent abort handles. Prioritize
   visible images, cap outstanding bytes and decoded pixels, decode to a useful
   display size, and cancel old decode work. The 256 MiB cache is a disk payload
   budget, not an RSS ceiling.
4. **Reader signal ownership needs lifecycle hardening.** The vertical
   adjustment owns a signal closure that captures ReaderUi, which strongly owns
   the scroller and thus the adjustment. This is a reference cycle capable of
   retaining article widgets, textures, and the navigation snapshot after the
   window closes. Use weak widget references or explicitly disconnect on
   teardown, then measure repeated open/close/logout cycles. This is a source
   finding; retained RSS was not measured in this pass.
5. **Storage occupies asynchronous worker threads.** Most repository async
   methods immediately lock SQLite and perform synchronous work. Slow queries,
   writes, and lock contention can occupy both backend workers, delaying
   network tasks. Use a dedicated serialized database worker or bounded
   blocking dispatch, with cancellation for superseded searches. Keep atomic
   offline intentions and durability guarantees.
6. **Scrolling still walks the document.** `position_from_offset` traverses
   preceding widgets on every adjustment change. The timer fix reduces
   wakeups, but not this per-frame traversal. Cache block geometry after layout
   or coalesce position calculation, while preserving positions during image
   insertion and article switching.

## Battery and idle behavior

The application has no recurring automatic sync interval in the inspected
code. Sync is user/startup driven, incremental, and guarded against overlap.
The list virtualizes visible rows, the reader displays only the first image
frame, HTTP image fetches share a client, and image cache hits avoid network.
These are useful existing efficiency properties. Removing completion polling
and abandoned scroll timers reduces avoidable wakeups while work is pending.

Actual idle CPU, wakeups, whole-process-tree RSS, GPU utilization, and battery
power have not been measured. There was no running Brooklet process during the
cache audit. Power savings cannot be quantified from source changes or parser
timings. Glycin decoder helpers must be included in process-tree measurements.

## Validation and repeatable checks

GNOME 51 SDK: 66 regression tests passed (55 core, 9 GUI, 2 contracts), the
separate ignored performance probe passed, GUI compilation and the smoke test
passed, and GUI/all-targets Clippy passed. Tests verify selection restoration,
retained object identity, no model events on an unchanged refresh, atomic mixed
changes, removal of a selected item, and copy-on-write isolation. Smoke testing
exercises the existing image/scroll-anchor checks, but is not interactive
pointer/touch verification. The test sandbox emitted settings/session-bus and
GPU initialization warnings; this is not proof of hardware rendering.

The follow-up passes 69 tests (57 core, 10 GUI, 2 contracts), all-targets GUI
Clippy, compilation, and the expanded GTK smoke test. New checks cover body-free
summaries, account isolation, body revision changes, executor responsiveness,
decode dimensions, rapid article replacement, construction over multiple
frames, saved-position restoration, and widget release after teardown. The
image smoke test decodes generated PNGs from an isolated temporary cache,
checks that distant images remain unloaded, and releases the session. No live
service or private article material is used. Glycin disables its additional
sandbox in the Flatpak development environment; this does not validate decoder
isolation in the installed app. Hosted CI passed for the published checkpoint.

With the SDK environment and vendored Cargo configuration set up as described
in the development workflow:

```sh
cargo test --features gui --bin brooklet large_list_refresh_performance \
  --locked --offline -- --ignored --nocapture --test-threads=1
cargo run --example parse_performance --locked --offline -- CORPUS.json
```

For a release decision, profile an optimized build using the default hardware
renderer. Exercise Inbox/Library scroll, a sync during scrolling, long articles,
wide tables, image-heavy articles, search typing, Previous/Next, Undo, and
repeated window teardown. Record frame-time distributions and missed frames,
including p95/p99 and worst stalls rather than average fps alone. Sample idle
CPU/wakeups for at least a minute and measure warm/cold RSS for the entire
process tree. Compare power on the same machine, screen brightness, and network
conditions. An idle app should sleep instead of generating 60 frames a second.
