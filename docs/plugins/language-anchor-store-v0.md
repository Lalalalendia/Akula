# LANG-01e — durable table-anchor sidecar

**Status: Draft, not GUI-ready.** This independent host-local app-data SQLite file stores only trusted note/table IDs and BLAKE3 full-table evidence. It never contains Markdown text or grants read permissions.

## Safety
Open requires a host-controlled app-data path. JSON payload version and size are checked. A single-row SQLite registry uses WAL, synchronous FULL and BEGIN IMMEDIATE with an exact revision compare-and-swap; competing writers cannot silently clobber anchors. Corrupt or unknown-format payloads are not reset and do not permit replacement.

**Important:** Collection membership lives in a separate RegistryStore at this stage, and cross-DB writes are NOT atomic. The trusted host must fail closed if the two stores disagree; an ID in either store is not permission. UI cannot be enabled until stable host document IDs and explicit reconciliation are wired to the active editor and per-note access.

## Tests
Seven unit tests cover durable reopen, absence of raw vocabulary in the DB, stale-writer conflict, malformed duplicate ID, unrelated database, corrupt/future-version payload and size bound.

## Deferred
An integrated migration to one atomic app-data state may replace these two stores. Never claim that a language table is active based only on an unverified source string or a matched fingerprint. Existing File Gate/DocumentHub retain all authority; no import or deployment.