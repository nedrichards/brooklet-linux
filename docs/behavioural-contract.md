# Behavioural contract

## Provenance

The initial oracle is the read-only checkout of
`https://github.com/nedrichards/brooklet-android` at commit:

`ac5dabb52790873b3104bf10dec80a26d64b1e4a`

The public `main` ref was verified on 2026-09-20. The available local checkout
had two later maintenance-only commits (an AGP update and unused-import cleanup),
which did not change the inspected parser, network, DAO, or sync contracts.
Neutral fixtures in
`tests/fixtures` record inputs and expected values; normal Rust builds do not
need the Android repository, Kotlin, Java, or Gradle.

The important inspected surfaces are `core-model` (`Models`,
`HtmlDocumentParser`, `SyncRules`), `core-network` (`MinifluxClient`, DTOs,
authenticated client policy, article image policy), `core-database`
(`Entities`, `BrookletDao`, migration and DAO tests), and `core-sync`
(`SyncEngine` and its tests).

## Product semantics retained

- Inbox means unread. Merely selecting an entry never changes read state.
  Deliberate activation opens it and marks it read.
- In an article list, `r` toggles the selected entry's read state without
  opening it. Marking it read dismisses it from Inbox and advances the list
  cursor; Undo can restore it without moving the reader.
- Opening an unread entry marks it read once. Keep Unread reverses that intent
  while preserving the open reader and its scroll position. Undo restores the
  unread row without reopening the reader.
- Read and starred changes are local-first and durable. The newest desired value
  replaces older pending work for the same account, entry, and field.
- Sync pushes pending Miniflux changes, then pending Karakeep work, before it
  pulls categories, feeds, and incrementally changed entries.
- A successfully sent local intention remains protected during the overlapping
  pull; acknowledgement follows page merge and cursor commit. A failed pull
  therefore leaves idempotent work queued rather than exposing stale state.
- Pulls use descending order, pagination, and a 60-second `changed_after`
  overlap. A stale remote value cannot overwrite an unacknowledged local value.
- Removed entries are deleted unless protected by a pending mutation or
  unfinished Karakeep delivery.
- Canonical URLs lowercase their origin, remove default ports and fragments,
  remove a trailing non-root slash, and retain query parameters.
- Unfinished Karakeep requests for one canonical URL coalesce. An explicit direct
  send retries a previously saved URL; Karakeep returns an existing bookmark if
  it is already present. Background sync does not replay completed saves.
  Successful receipts remain visible for 30 days before pruning. Direct delivery
  requires a 200/201 response identifying the requested link before acknowledging it.
- Ordinary read articles default to 30-day retention with an upper bound of
  5,000. Unread, starred, recently opened, pending-mutation, and pending-
  Karakeep entries are protected.

## Reader document model

Original HTML remains authoritative. Parsing produces headings (level, text,
inline semantics and links), paragraphs, quotes, code, ordered or unordered list
items, captions, tables, and images.

The compatibility fixtures cover:

- named, decimal, and hexadecimal entities, decoded once only;
- nested images and `data-src`;
- single-quoted, double-quoted, and unquoted attributes;
- loose `<br>`-separated feed prose and Phoronix-style inline links;
- safe relative links resolved against the article URL;
- strong/bold, emphasis/italic, inline code, and line breaks;
- ordered list sequence, `<ol start>`, and `<li value>`;
- figures/captions and simple header/data-cell tables.

Unsupported tags do not grant arbitrary styling or execution. Script, style,
event-handler, and article CSS content is not rendered. Inline output is a typed
allow-list that the GTK reader maps to Pango attributes.

## HTTP contracts

Validation calls `GET /v1/me` and `GET /v1/version` with `X-Auth-Token` and
`Accept: application/json`, and rejects Miniflux versions older than 2.3.2.
Miniflux requires HTTPS. Karakeep endpoints accept HTTP and HTTPS. Both reject
embedded credentials and do not follow redirects.

Entries use `GET /v1/entries` with `direction=desc`, explicit ordering, limit,
offset, and optional `changed_after` or status. Entry lookup uses
`GET /v1/entries/{id}`. Read/unread and star changes use `PUT /v1/entries` with
`entry_ids` plus `status` or `starred`. Third-party save uses
`POST /v1/entries/{id}/save`, refresh uses `PUT /v1/feeds/refresh`, and subscribe
uses `POST /v1/feeds` with `feed_url` and optional `category_id`.

Unknown response fields are ignored. Status classification is:

- 401/403 authentication;
- 400/404/422 malformed and non-retryable;
- 408/429/5xx retryable;
- TLS/certificate failures certificate-class;
- other network/transport failures retryable.

## Explicit Linux changes

Android-specific mechanisms are not contracts. Room becomes `rusqlite`,
WorkManager becomes application-lifetime Tokio work, Android Keystore becomes
`oo7`, Compose lists become GTK list models, and Android share becomes Copy Link.
The databases are not binary-compatible and the applications coordinate only
through Miniflux.
