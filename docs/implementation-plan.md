# Implementation plan

Each step is intended to be one focused, reviewable commit.

## Phase 0 — toolchain and shell

1. `chore(repo): establish project policy and licence` — repository metadata,
   GPL-3.0-or-later, ignore rules, README, architecture, behavioural provenance,
   and this plan.
2. `build(gnome): add Cargo Meson Blueprint integration` — one Rust crate,
   compiled GResources, desktop/AppStream data, GNOME 51 Flatpak manifest, and
   Builder-discoverable layout.
3. `feat(shell): add adaptive Brooklet window` — Inbox/Saved/Library view stack,
   list-pane destination switcher, placeholder list/reader split, breakpoints, semantic actions,
   and an empty-state launch smoke test.
4. `ci: validate Rust GNOME and Flatpak builds` — format, Clippy, tests, Meson,
   Blueprint, desktop/AppStream validation, and clean-checkout Flatpak build.

## Phase 1 — portable contracts

5. `feat(model): define GTK-independent Brooklet domain` — account, feed,
   entry, document blocks/inline spans, mutations, delivery, cursor, reader
   position, sync state, and service traits.
6. `feat(reader): parse cached HTML into native blocks` — HTML5 parsing,
   allow-listed inline semantics, image/caption/table/list handling, URL
   resolution, and Android-derived JSON fixtures.
7. `feat(network): add URL and image destination policy` — service validation,
   canonicalisation, public-address classification, connector-bound DNS checks,
   a 20 MiB cap, and security tests.
8. `feat(miniflux): implement authenticated API contracts` — redirect-free
   rustls client, version validation, categories/feeds/entries/pagination,
   mutations, refresh/save/subscribe, typed failures, and local mock tests.
9. `test(contract): lock initial cross-language behaviour` — cursor overlap,
   retention, retry classification, canonical URLs, payload/path fixtures, and
   provenance review.

## Phase 2 — durable sync

10. Add embedded SQLite migrations and an in-memory migration harness.
11. Implement repository queries and atomic mutation/Karakeep coalescing.
12. Implement merge/removal/retention rules and deterministic repository tests.
13. Implement the trait-driven SyncEngine and fake-service ordering/error tests.
14. Add the Tokio backend command/event loop and GTK controller bridge.

## Phase 3 — first usable release

15. Add `oo7` setup validation and account persistence without secrets in DB.
    This slice is brought forward during Phase 1 so manual server testing is
    available for every subsequent vertical slice.
16. Bind startup/manual sync and durable status to the shell. Cached-first
    startup, incremental cursors, removal, and durable error records are in place.
17. Implement virtualised Inbox selection versus activation.
18. Implement local-first Mark Read, Mark All Read groundwork, toast Undo, and
    Ctrl+Z.
19. Implement the clamped native text reader, link launching/copying, Keep
    unread, reader position, and previous/next context.
20. Exercise the Flatpak against a real Miniflux server and fix only evidenced
    interoperability issues before tagging the first usable release.

## Integrated application slice

Saved, Library drill-down/search, star, subscribe, previous/next, Mark All Read,
durable Karakeep delivery, the connector-bound image client, Glycin, retention
preferences, and About/diagnostic surfaces have been integrated. The remaining
release gates are real-server end-to-end use, visual/a11y review, responsive
tuning, and translation/release polish. These should not postpone using the app.
