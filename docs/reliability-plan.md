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
4. Expose and recover failed Karakeep deliveries; validate direct settings.
5. Refresh visible Library drill-down pages while preserving context.
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
