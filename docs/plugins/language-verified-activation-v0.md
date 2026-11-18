# LANG-02c — verified union from non-atomic metadata stores

**Draft.** Collection membership and table anchors currently persist in two independent SQLite files. A membership is NOT sufficient to produce an exercise; the host must provide complete, authorized current note snapshots, and only unique, unchanged, same-note table anchors can join them.

New pure `learning::activation::assemble_verified` validates both registries, deduplicates requested sources, matches each table against the complete current set of tables in its note, then delegates eligible rows to `collections::assemble`. Any missing, edited, duplicated or unverified source is returned as unavailable, never given an old table ID or text. It does not perform IO, authenticate a vault or control permission grants.

**Caller contract:** trusted host must verify note identity, permissions, current revision and all tables in the note; recheck both sidecar DB revisions if they can race after reading. Do not pass only the requested table, which could conceal another identical table. No installed UI or cross-db transaction exists yet.

Eight tests cover 4 tables / 3 notes / 2 collections, updated table HOLD, duplicate content HOLD, missing/denied source, foreign vault, duplicate live note, corrupt anchors and table bound.

Risk: this is a fail-closed prototype, not production authority; future trusted host integration and installed GUI E2E must prove note identity under rename/move and dirty editor edits.