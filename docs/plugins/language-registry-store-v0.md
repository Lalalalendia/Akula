# LANG-02b — durable language collection metadata (first-party host)

Status: draft Core implementation. Parent [PR #1005](https://github.com/Lalalalendia/Akula/pull/1005).

## What is stored

One versioned, bounded app-data SQLite registry containing collection names, table identities and explicit column mappings. A collection may reference tables from several notes and may share sources with other collections. The database stores no note text, word, translation, media, table cell content or study history.

## Safety contract

- The **trusted host** must select a dedicated app-data path; never accept a guest supplied path. This is not a vault File Gate or a plugin permission broker.
- Rows/source IDs and collection metadata are **not capabilities**: reading original notes requires a fresh authorized DocumentHub snapshot and verified File Gate permissions.
- SQLite WAL, synchronous FULL and an immediate transaction with compare-and-swap revision provide durable and conflict-aware replacement. Two stale instances cannot silently overwrite one another.
- The store refuses unfamiliar schema versions, unknown fields, corrupt metadata, duplicate membership and oversized records without silently reverting to defaults or modifying the source Markdown.
- A future UI must retain the loaded revision, display conflicts instead of retrying a stale save automatically, and expose export/backup. A separate reconciliation layer must mint and verify stable note/table identities.

## Remaining work before LANG-02 acceptance

Connect trusted host app-data lifecycle, issue stable note/table IDs through host, resolve table snapshots from live unsaved document buffers, add editor command and collection selection UI, test persistence across installed Windows restart, confirm read permissions and denied scopes. Draft Core tests alone do not establish user-facing completion.

Test: `cargo test -p aquilum-core learning::store::tests --locked` and full exact-head application/protected CI.