# Reliability work plan

Work through these checkpoints in order. Each checkpoint gets regression tests,
the relevant existing checks, and a focused semantic commit before the next starts.
Preserve pre-existing uncommitted changes. Do not publish or release as part of
this plan.

1. **Complete:** Subscription response handling: accept Miniflux's `feed_id` response and
   retain the existing post-subscription sync. Test the real response shape,
   category payloads, and failure handling.
2. **Complete:** Separate upload failures from incoming sync: preserve pending intentions
   while allowing incoming articles during delivery failures.
3. **Complete:** Repair Miniflux credentials without clearing local data.
4. **Complete:** Expose and recover failed Karakeep deliveries; validate direct settings.
5. **Complete:** Refresh visible Library drill-down pages while preserving context.
6. Follow up asynchronous server feed refresh with a bounded incoming pull.
7. Show persistent, actionable sync and delivery health.
8. Reconcile remote deletions and test concurrent pagination changes.
9. Retain and display feed parsing errors and disabled state.
10. Show recoverable startup database failures without destroying the database.

Add end-to-end coverage alongside the relevant checkpoints for subscription,
offline edits and restart/reconnect, credential replacement, integration outage,
remote deletion, and refresh inside a feed.

## Verification log

- Initial review: source inspection and Miniflux API verification; no live-account
  test. Existing automatic refresh and outgoing delivery work remains in the
  working tree and is outside checkpoint 1's commit.
- Checkpoint 1: creation response now returns the feed ID; no metadata request
  can misreport successful creation as failure. Mock HTTP regression cases cover
  the real 201 response, categories present/absent, server rejection, malformed
  responses, and no additional request after creation.
- Checkpoint 1 verification: baseline 86 unit + 2 contract tests; updated working
  tree 88 unit + 2 contract tests; isolated commit 62 unit + 2 contract tests.
  SDK formatting, GUI Clippy with warnings denied, GUI build, and GUI smoke
  passed. Smoke emitted development-runtime portal/session-bus and Glycin
  warnings. Live-account subscription was not exercised.
- Checkpoint 2: service delivery failures no longer abort incoming sync. Unsent
  read/star intentions remain protected and Karakeep retries remain queued.
  Separate durable delivery status survives a successful pull and clears on
  delivery recovery. Local storage failures still abort. This commit includes
  the diagnostic migration and absent-cursor handling needed for first sync;
  the unrelated outgoing worker and mutation revision work remains unstaged.
- Checkpoint 2 verification: four new regressions failed before the fix. The
  final six integration cases cover read/star failures, partial delivery,
  Karakeep outage plus feed refresh, recovery, combined errors, local database
  failure, and first-sync bootstrap. All 96 working-tree tests and 72 isolated
  commit tests passed, including diagnostic migration tests. SDK formatting,
  isolated Clippy, GUI Clippy with warnings denied, GUI build, and GUI smoke
  passed. Smoke emitted development-runtime portal/session-bus and Glycin
  warnings. Real-service outage and recovery were not exercised.
- Checkpoint 3: Preferences now offers Reconnect for the existing server and
  username. Validation precedes replacing only the Miniflux keyring item; cached
  data, pending work, reader positions, Karakeep settings/secrets, retention and
  sync status stay intact. Missing old secrets can be repaired without loading
  them. Setup cannot overwrite an account. Controller account operations are
  serialized with logout, and successful reconnect triggers one sync after any
  old in-flight request completes.
- Checkpoint 3 verification: all 103 working-tree tests and 79 isolated-commit
  tests passed. Seven new integration tests cover replacement, missing tokens,
  validation/identity/keyring failures, retained data after reopen, setup
  overwrite rejection, logout, and resumption of persisted outgoing work.
  Working-tree and isolated SDK formatting, Clippy with warnings denied, GUI
  build and GUI smoke passed. Reconnect UI smoke exercises the displayed form,
  blank-token rejection, duplicate submission, failure/retry, controller/logout
  ordering, and deferred sync exactly once. Smoke emitted development-runtime
  portal/session-bus and Glycin warnings. A real account/keyring repair was not
  exercised.

- Checkpoint 4: Preferences offers a list of unfinished Karakeep deliveries with
  their route, state, URL and error. Retry deliberately uses current settings;
  dismiss removes only the unfinished local receipt. Account-scoped recovery
  cannot replay or remove a successful receipt and preserves articles and edits.
  Recovery/settings serialize with uploads and logout. Missing direct credentials
  now record an actionable receipt error without preventing integration delivery.
- Direct settings require a key and a read-only, bounded GET of the bookmarks
  endpoint before persistence. Blank keys retain and revalidate the saved key.
  Validation/keyring failures preserve settings; a failed database save restores
  the previous Karakeep secret without touching Miniflux. Keyring/database writes
  cannot form a cross-store transaction, so rollback errors are reported.
  The payload omits the unnecessary custom source field, and an arbitrary HTTP
  409 is no longer accepted as proof of delivery. API references:
  https://docs.karakeep.app/api/list-bookmarks/ and
  https://docs.karakeep.app/api/create-bookmark/.
- Checkpoint 4 verification: all 110 working-tree and 86 isolated-commit tests
  passed. Seven new regressions cover read-only authentication/response checks,
  redirect/conflict rejection, restart-visible failures, rerouting/retry,
  account-scoped dismiss, saved-receipt protection, missing keys, keyring failures
  and database-write rollback. SDK formatting, GUI Clippy with warnings denied,
  GUI build and GTK smoke passed in both trees. UI smoke exercises actual retry,
  duplicate clicks and dismiss through the controller against SQLite. Smoke
  emitted development-runtime portal/session-bus and Glycin warnings. Real
  Karakeep/Miniflux integration and system keyring recovery were not exercised.

- Checkpoint 5: Dynamic feed/category pages now retain their query scope and
  widget/model state for their navigation-stack lifetime. Visible pages reload
  after sync, local updates and navigation/Back through the shared refresh path.
  Feed reloads preserve ID-based selection and the existing article viewport
  anchor; category reloads reuse feed rows and preserve focused feed/viewport.
  Empty pages can receive new content without being reopened. Refresh errors keep
  cached content. Generation and visible-page checks reject stale/hidden results;
  popped pages and window teardown release state and cannot be resurrected.
- Checkpoint 5 coverage: GTK scenarios cover inserted/updated articles, renamed
  and inserted feeds, viewport/focus/selection preservation, out-of-order loads,
  failure retention, navigation during a request, hidden destinations, Back,
  empty/nonempty transitions and page cleanup. A full production-window journey
  runs the actual Library and Sync actions through AppController and real SQLite,
  verifying new articles, saved-state changes, no navigation-driven read writes,
  stable selection/scope and updated category metadata on Back. This journey is
  included in both GTK smoke and the required keyboard regression entry point;
  with the existing X11 driver it also checks Up/Down after feed refresh.
- Checkpoint 5 verification: all 110 working-tree and 86 isolated-commit Rust
  tests passed. SDK formatting, GUI Clippy with warnings denied, GUI build, full
  GTK smoke and keyboard logic checks passed in both trees. Physical X11 events
  failed with `Physical Down failed from header` in the unchanged committed
  baseline as well as the working tree; the added post-refresh physical check
  also could not be verified on this display. X11 diagnostics confirmed the
  intended test-window focus; the event driver did not reach the app controller.
  No new keyboard-routing change was made. Development-runtime portal/session
  bus, accessibility-bus and Glycin warnings were emitted during these checks.
  Live-service and manual pointer/touch behavior were not exercised.
