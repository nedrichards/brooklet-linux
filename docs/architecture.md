# Architecture

## Purpose and boundaries

Brooklet is a native GNOME expression of the existing Brooklet product, not a
port of its Android mechanisms and not a general-purpose feed manager. Miniflux
owns feeds and synchronised read/star state. Brooklet owns the local Inbox
workflow, offline cache, native reading, reader position, pending intentions,
and optional Karakeep delivery.

The Android repository is a read-only behavioural oracle. This repository does
not link to it, copy runtime source from it, invoke Gradle in normal development
or CI, or share its Room database. Behaviour is reimplemented from documented
contracts and neutral fixtures.

## Process and thread model

```text
GTK main thread
  AdwApplication / windows / GObjects
             |
             v
       AppController
   typed commands | events
             |
             v
dedicated Tokio runtime
  SyncEngine -- Repository
      |       -- SecretStore
      |       -- Clock
      +---------- MinifluxApi
      +---------- KarakeepApi
```

GTK widgets and all GObjects stay on the GTK main thread. Domain values contain
no GTK types. The controller submits owned commands to a dedicated Tokio
runtime and receives typed events back through a main-context channel. Widgets
do not open databases or make HTTP requests.

## Account setup vertical slice

Setup is intentionally implemented before the rest of Phase 3 so every later
sync and Inbox slice can be tried with a real account. On first activation the
controller asks the repository for account metadata. If none exists, the main
thread presents an adaptive `AdwDialog`; validation and persistence run on the
dedicated Tokio runtime.

The submitted value crosses the UI boundary as an owned `SetupRequest`. The
password row is cleared immediately, and the request is not printable through
`Debug`. `AccountSetupService` first applies the configured-service URL policy,
then validates `/v1/me` and `/v1/version`. It stores the token through
`SecretStore` and writes only the confirmed server URL, username, and version
to SQLite. If metadata persistence fails after secret creation, the service
removes the newly stored secret so setup is not left half-configured.

`IdentityValidator`, `Repository`, and `SecretStore` are injected boundaries.
Automated tests therefore cover the ordering and separation rules without a
desktop keyring, network access, or real credentials; the existing Miniflux
mock-server tests independently cover the validation HTTP contract. The launch
smoke test constructs both Blueprint resources inside the GNOME runtime.

Sync now pushes local intentions, merges categories and feeds, paginates changed
entries of every status with cursor overlap, handles removals, and applies
retention after a successful pull. On startup cached rows are presented before
network work, so loss of connectivity does not blank Inbox, Saved, or Library.
The initial unread-snapshot repository method remains for migration-era tests.

Automatic sync uses the same incremental path, without requesting a server-side
feed refresh or prefetching article images. While the main window is active,
cached content becomes eligible for refresh five minutes after the last completed
sync. Startup reuses the persisted last-success time. A coalesced, sixty-second
GLib timer checks wall-clock age so a long suspend counts as elapsed time, even
if window activation never changes. Activation and network-property changes
also check immediately. Inactive windows have no refresh timer; returning to
a stale window triggers one sync, without replaying missed intervals.

Automatic requests wait while GIO reports an unavailable or metered network,
or a sync is already running. This allows private Miniflux servers on networks
without public Internet connectivity. Failures retry after five, ten, twenty,
then at most thirty minutes; automatic failures remain in sync status without
repeated error toasts. Manual refresh bypasses these gates and a successful sync
resets backoff. Timers and network handlers are removed when the window closes.
The network gate depends on the desktop reporting its metered status.

The Inbox list uses a `GtkListView` with ordinary GTK box/label children; rows
do not use `AdwActionRow`, whose supported parent is `GtkListBox`. Selection is
held by `GtkSingleSelection` and has no data-layer side effect. The separate
activation signal parses the cached HTML and populates the native reader, then
asks `AdwNavigationSplitView` to show content when collapsed. GTK's list cursor,
keyboard focus, and the reader's active article are separate: J/K and arrow
keys move the cursor without opening an article. Enter, a mouse double click,
or a touch tap activates it. A single mouse click only selects the row. The
window routes an initial movement key to the visible article
list, so keyboard browsing does not depend on first focusing the sidebar.
After cursor movement, Enter activates the selected row even if the prior
header control still held focus; Tab returns Enter to normal control activation.
Pointer hover does not change the cursor. Read state and one
durable, coalesced pending mutation are written immediately. On wide windows
the active row remains visible as Read until the cursor leaves it; on collapsed
windows it is removed as the reader takes over. The Inbox row action switches
between Mark Read and Mark Unread, and Undo restores the row.

`Repository`, `MinifluxApi`, `KarakeepApi`, `SecretStore`, and `Clock` are
testable boundaries. The production repository uses one `rusqlite` database;
tests can use an in-memory implementation. Thin GObject adapters such
as `EntryObject` expose immutable snapshots to `gio::ListStore` without making
the domain model a GUI model.

## UI composition

The root is one `AdwNavigationSplitView`: its sidebar is the current collection
or Library drill-down and its content is the stable reader. A bottom
`AdwViewSwitcherBar` inside the sidebar changes its `AdwViewStack` between
Inbox, Saved, and Library without reserving a second navigation rail. Library
uses `AdwNavigationView` for category, feed, and article drill-down.

The Blueprint breakpoint near 800sp collapses list and reader into native
navigation, so opening an article slides the reader over the collection and the
header's back affordance returns to it. The destination switcher remains part
of that collection pane rather than occupying reader width.

Cursor movement and activation are separate state transitions. Scrolling the
viewport does not move keyboard focus or open an article. Activation opens the
entry and emits exactly one mark-read intention when appropriate. `Keep Unread`
is available in the reader header, menu, and with U. It leaves the reader and
scroll position intact. If an open operation is still marking the article read,
it suppresses that result and restores the unread intention afterward.

`AdwToastOverlay` owns the current reversible local action. Mark Read and Mark
All Read update the repository first, update the list immediately, and then
offer Undo. Opening an unread article in any collection also offers Undo.
`win.undo` is the single semantic action behind Ctrl+Z, the toast button, and
`u` in the same article-list and reader contexts as `r`; it restores read state
without reopening the reader. Single-key actions ignore text fields and other
editing controls. Keep Unread remains available in the reader header and menu.

## Data and consistency

SQLite is the UI source of truth. A local read/star action and its pending
mutation are written in one transaction. The pending-mutation key is
`(account_id, entry_id, field)`, so a later desired value replaces the earlier
one. Local actions enter a FIFO persistence queue before backend tasks are
spawned, preserving invocation order. After a successful local commit, a watch
channel wakes an independent delivery worker. It debounces for two quiet seconds,
capped at ten seconds from the start of a burst, and never cancels an upload
when more actions arrive. Startup checks persisted work even if article refresh
is still fresh. Window inactivity and metered connections do not gate delivery.
Retryable failures back off exponentially from five seconds to thirty minutes;
other failure kinds retry every thirty minutes. New actions do not shorten an
existing failure backoff. A successful drain resets it. Manual sync can drain
the queue immediately. Closing or quitting waits for queued local commits;
network delivery resumes at next launch if unfinished when the process exits.

Both standalone delivery and full sync push pending read/star work before
Karakeep work. Full sync does this before requesting a server feed refresh.
Each accepted batch is acknowledged immediately, before further requests or
downloads can fail. Acknowledgement matches both the desired value and the
monotonic revision stored in `updated_at_ms`; revisions increase even when rapid
changes share a millisecond or the clock moves backwards. This protects newer
equal values as well as reversals made while a request is in flight.

Uploads, disconnect, and each article fetch-and-merge share a mutex. A stale
response fetched before an upload is merged while the local intent is still
pending; a fetch after acknowledgement imports the current server state,
including changes from other clients. Uploads can run between pages rather than
waiting for a whole article refresh. Miniflux and Karakeep requests have a
ten-second connection timeout and a thirty-second total timeout.

Remote merge preserves fields with unacknowledged local intentions. Incremental
entry pulls overlap the last successful `changed_after` cursor by 60 seconds;
the cursor advances only after all pages merge successfully. A removed remote
entry remains local while it has pending mutation or unfinished Karakeep work.

Original article HTML is retained. Reader blocks are parsed when an article is
opened on a bounded backend task. List queries return body-free summaries with
a content revision; activation fetches the account-scoped body. Hidden Library
views load when mapped. Native document construction is spread across frame
callbacks, including separate table-cell tasks. Parsed output is never the
source of truth, so parser fixes apply to already cached articles. Inline spans
are rendered through an escaped Pango
allow-list. Reader position is stored as a block index plus offset.

The schema advances through numbered SQL migrations embedded in the executable.
Migration and reopen tests exercise in-memory and file-backed databases.
Connections enable foreign keys and WAL.
Repository operations acquire a single asynchronous permit before executing
synchronous SQL on a blocking thread, preserving transaction boundaries without
occupying the asynchronous backend workers. Opening and migrations happen at
startup.

## Networking and trust zones

Configured Miniflux and Karakeep origins form a credential-bearing trust zone.
Miniflux URLs must be HTTPS; Karakeep endpoints accept HTTP and HTTPS.
Neither allows user info, and both clients disable redirects.
Tokens are added only at request construction and are never included
in errors or tracing fields.

Article images form a separate untrusted zone with a separate client and DNS
resolver. It accepts only HTTPS, follows no redirects,
disables ambient proxy use, caps responses at 20 MiB, and gives the connector
only addresses that have already passed public-address policy. This avoids a
check-then-resolve DNS rebinding gap. Glycin decodes bounded response bytes
without creating a persistent image file; alt text remains on failure. Compressed
bytes use a disposable bounded SQLite cache. Reader sessions fetch and decode
near the viewport, bound concurrent jobs and decoded payloads, cancel obsolete
work, and evict offscreen textures. See the image policy in `reader-testing.md`
for limits. Signal handlers use weak widgets or disconnect when their owner
closes.

## Credential repair

Preferences offers a reconnect dialog with the existing server and username
fixed. `SetupService::reconnect` validates a replacement token against that
identity before replacing only the Miniflux keyring item. It neither loads the
old token nor writes account metadata, and preserves article HTML, reader
positions, pending mutations, Karakeep configuration and credentials, delivery
work, retention preferences, and sync status. Setup refuses to overwrite an
existing account. Identity mismatch or validation failure never writes the
replacement credential; secret-store failure leaves local data available for
retry.

The controller serializes setup, reconnect, and logout with an account-operation
mutex. Logout therefore cannot clear an account while reconnect is validating
and then leave a newly written token behind. The dialog clears its password row
on submit, prevents duplicate submission and closing while validation runs, and
returns to an editable form after failure. Success triggers one manual sync,
waiting for an old in-flight sync to re-enable its action if necessary.

## Secrets

Service secrets live behind `SecretStore`; the production implementation uses
`oo7` and its sandbox-aware backend. SQLite stores an opaque account identity
and non-secret configuration only. Secret values are short-lived in setup UI
state, never accepted on command lines, and excluded from diagnostics.

## Errors and diagnostics

Internal layers return typed `thiserror` errors. Application boundaries add
user-facing context without retaining credentials or article bodies. `tracing`
records phases, counts, durations, endpoint classes, and stable identifiers;
it never records tokens, API keys, authenticated headers, or full article HTML.
Offline and retryable failures update sync state while leaving cached UI usable.
Delivery errors are stored separately from article refresh errors and clear on
successful delivery without hiding an unresolved refresh failure. A status row
created by an error before the first successful pull has an absent cursor and
triggers bootstrap normally.

An accepted write cannot be replayed merely because a later batch or download
failed. A lost HTTP response or a process crash between server acceptance and
SQLite acknowledgement remains ambiguous: durable pending work is retried.
Read/star requests set explicit values; third-party save requests do not offer
an exactly-once delivery guarantee.

Incoming sync continues when read/star upload or Karakeep delivery fails at a
service boundary. Unsent intentions remain queued and protected from remote
merge. Delivery errors are stored separately from refresh errors: a successful
pull advances freshness without concealing unsent work, and a later successful
delivery clears only its delivery error. Local database or filesystem failures
still stop sync. Missing Miniflux credentials stop before the pull because the
same credentials are required to fetch articles.

## Packaging

Meson owns resources, metadata, installation, and the Cargo invocation.
Blueprint compiles to GtkBuilder XML before `glib-compile-resources` bundles it.
Cargo owns Rust dependency and test builds. Flatpak is canonical and targets
GNOME Platform/SDK 51 with network access and no broad host filesystem access.
