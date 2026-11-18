# LANG-02d — unified atomic collection + table anchor state

- New Core module `learning::state` is the **only intended writable store** for first-party language collections. Existing experimental `learning::store` and `learning::anchor_store` remain non-integrated for prototype tests; they must not be opened or merged automatically.
- One bounded versioned SQLite row contains `StudyState { registry, anchors }`; one `BEGIN IMMEDIATE` revision-CAS transaction binds a table ID, one per-table column mapping, and named collection membership.
- The trusted native host must supply its existing workspace/document UUIDs, a **complete** currently authorized live note table array and a host-minted random table UUID. Never use a byte offset, a guessed title or the fingerprint as an ID.
- Unique unchanged tables reattach safely after reordering; changed/duplicate tables go HOLD. Conflicting mapping and stale revisions reject with no partial changes.
- No Markdown text or vocabulary is persisted. Unknown/foreign schema, corrupted JSON and orphan membership reject without silent reset. No existing app-data state is migrated automatically.
- This is a Core contract only until native GUI and full-note enumerator are integrated. Requires exact-head Rust and Windows checks. No deployment.