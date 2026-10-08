# SQLite efficiency follow-up

The disposable libgom backend prototype was removed. Brooklet retains rusqlite;
there are no new dependencies. The libgom query/list models at upstream revision
`ebc850c2be6534d2472cc61cb0e0476f165f88b2` inspired demand-driven materialization,
stable row objects and bounded concurrent refresh work.

## Changes

* Article lists initially load 128 body-free summaries and fetch further keyset
  pages near the viewport or keyboard selection. Timestamp plus ID ordering
  resolves ties. Refresh retains the visited prefix; append retains existing GTK
  row objects. Reader Previous/Next deliberately loads the full summary snapshot
  on activation, and Mark All Read/Undo includes unread articles beyond loaded pages.
* Each view allows one active reload and retains only the latest queued reload.
  Generation checks reject obsolete results; weak UI references avoid retaining
  closed windows.
* The connection retains at most 32 prepared SQL statements. This caches plans,
  not query results, so local mutations and sync need no result-cache invalidation.
* Identical remote rows skip SQLite updates, while pending local read/star
  intentions still take precedence. Content and nullable-author changes remain
  detectable. Ordering indexes cover Inbox, Saved and feed keyset reads without
  temporary sorting.

The existing single database connection remains. Pooling should follow evidence
of residual contention. Additional body/parsed-content or result caches would
retain more memory and require revision/account/mutation invalidation; none were
added. Loaded list prefixes can grow as the user browses; this is demand paging,
not a fixed memory ceiling.

## Measurement and validation

`cargo run --offline --locked --example sqlite_efficiency` uses synthetic data,
warms both paths and alternates 12 passes; setup is outside the measured section.
GNOME SDK 51 debug-build medians on this machine:

| Work | Existing path | Changed path |
| --- | ---: | ---: |
| 10,000 identical point reads | prepare: 124.058 ms | prepare_cached: 42.344 ms |
| 20,000 body-free summaries | full read: 74.626 ms | first 128: 0.838 ms |

The first comparison isolates statement preparation. The second compares initial
work of different sizes, not the cost of eventually reading every page. Neither
establishes frame-rate, battery, or whole-application improvements.

Default Rust tests, GUI compilation, all-target GUI Clippy, and GTK paging,
keyboard and production-window journeys were checked. Regression coverage includes
tied cursors, account/view filters, changing datasets, coalesced/stale loads,
selection preservation, teardown, bulk read/Undo beyond the loaded prefix,
migration rollback/upgrades and unchanged remote rows preserving local intentions.

The broad launch smoke test reached the reader/action regressions but failed its
cached-gallery check: four image decode failures and zero loaded images. Full
launch smoke is therefore not green; this check does not establish image-path
health. Focused paging and keyboard journeys are reported separately.
