# ADR-SYNC-022 — Crash-durable participant enrollment and cold-Core File Gate HOLD

Status: first conservative P0 slice; **not** a complete mutation-authority proof.

## Threat / gap

Even after ADR-SYNC-021, a fresh Core starts with an empty `guarded_editors` registry. Unless the app-data remembers that the workspace was previously enrolled, `gate::write` can miss the explicit S3 route and silently use the ordinary, unlocked File Gate. The SQLite `write_intents` table only covers individual DocumentSession publication attempts; it cannot identify a whole previously participating workspace.

## This change

- Documents SQLite v2 → v3 adds **insert-only** `guarded_participants(root, vault_id, profile_db, custody_db, custody_scope)`. The v1 migration creates both v2 intents and v3 enrollment, preserving old CRDT replicas, updates and sync points. A fresh v3 database has both tables. A conflicting already enrolled identity is refused without UPDATE/REPLACE.
- `activate_registered_s3_editor` verifies the independently bound S3 profile, physical custody identity and pre-existing OS lock, arms durable prewrite evidence with no open CRDT editor, then **FULL-sync commits the participant record before exposing any live guarded File Gate route**.
- `DocumentHub::new` loads enrollment on Core startup. A nonempty participant registry, unreadable schema, malformed rows or unavailable DocumentStore establishes a conservative process-local File Gate HOLD. This is deliberately **not** inferred from a missing record, and never auto-clears.
- `Core::file_mutation_guard_for_write`, used by ordinary File Gate write/create/binary copy/folder/tree/rename/trash/restore/retention and Core trash purge, now denies all such writes if participant HOLD is active. **Every workspace managed by this Core** is temporarily covered, not just the registered root, until path-specific guarded-or-denied routing is independently proved.
- The narrow `file_mutation_guard_for_custody_write` exception is used only inside `write_checked_history`, which the verified S3 writer invokes while holding one physical OS lock across the primary write and HistoryService. The same history relocation recovery checks remain mandatory. A saved Note still additionally requires its exact, one-use DocumentSession ticket and FULL-synced write intent.
- A fresh Core with the same app-data cannot silently downgrade an enrolled vault back to ordinary mutation; tests check restart, attempted note overwrite/new note and another workspace's create, and corrupt Documents.sqlite3 failure.

## Exact limits / required next gates

This is a **cooperative** appdata marker. Deleting the *entire* appdata/registry, or using a separate unregistered appdata profile for the same physical root, can remove the Core's local evidence; the record does not authenticate hostile same-user processes or replace a broker-owned physical lock namespace. Crash durability of Windows native primary-file replacement has not been proved. The global HOLD is intentionally conservative and may block unrelated workspaces in the same Core. Direct HistoryService operations, plugin/OS writes and any mutation path that bypasses `file_mutation_guard_for_write` require explicit inventory and coverage before production. In particular, no permission to rename/trash/restore S3-controlled files has been added.

**No public automatic S3 sync, overwrite/delete, auto-baseline advance, GC, operator clearance or user-facing release.** Next P0 is path-scoped mutation auditing plus redundant cold-start enrollment evidence, independent appdata identity detection, and kill/restart tests across two physical processes.
