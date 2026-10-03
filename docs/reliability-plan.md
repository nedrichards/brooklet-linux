# Reliability work plan

Work through these checkpoints in order. Each checkpoint gets regression tests,
the relevant existing checks, and a focused semantic commit before the next starts.
Preserve pre-existing uncommitted changes. Do not publish or release as part of
this plan.

1. **Complete:** Subscription response handling: accept Miniflux's `feed_id` response and
   retain the existing post-subscription sync. Test the real response shape,
   category payloads, and failure handling.
2. Separate upload failures from incoming sync: preserve pending intentions
   while allowing incoming articles during delivery failures.
3. Repair Miniflux credentials without clearing local data.
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
