# ADR-SYNC-021 — Guarded editor requires a matching crash-durable intent

Status: opt-in, internal integration between merged #215 (S3-registered File Gate writer) and #225 (DocumentHub write-intent journal).

## Gap closed

The two features were independently tested and merged, but merely enabling the #215 File Gate route did not require the #225 appdata SQLite intent. A standalone native Note or MCP writer using the same `gate::write` function could therefore publish under the correct OS lock *without* an intent that would hold it across a crash, while `DocumentSession` could be armed separately.

## Enforced order

1. Host explicitly verifies the pre-existing S3 profile, registered physical workspace root and external custody SQLite/OS lock. No implicit S3 access or auto-bootstrap.
2. While the Core guarded-editor registry remains locked, `activate_registered_s3_editor` **must call** `DocumentHub::arm_durable_intents_for_root` before making its guarded route visible. It refuses if the appdata journal is unavailable or live CRDT editor sessions already exist. The root remains opt-in and is not automatically activated after restart.
3. `DocumentSession` commits a single FULL-synced `write_intents(key, base_hash, proposed_hash)` row before entering `HubIo::write`.
4. After the FULL-synced SQLite INSERT, the same DocumentSession thread arms a **one-shot in-memory ticket** carrying the exact Core-instance UUID, `identity(path)`, old digest and proposed digest. The ticket is removed on every guarded File Gate result, denial or panic; it is never persisted or reconstructed from SQLite. Reconstructing a different Core on the same thread creates a distinct UUID.
5. Every guarded `files::gate::write` checks **both** the one-shot same-Core ticket and its independently read, matching durable SQLite intent. A stale journal row after an ambiguous write, Core restart or crash cannot authorize a replay, even when old/new content hashes still match. **If the ticket exists but activation is incomplete**, File Gate also refuses the otherwise ordinary legacy-write fallback; token consumption happens even when root selection fails. Missing/mismatched evidence is a **prewrite denial**, not fallback.
6. The guarded writer then rechecks profile/custody binding, holds the same external OS lock through File Gate and HistoryService, and reports ambiguous results as NeedsReview.
7. Only `DocumentSession` may remove the exact intent after confirmed file+history and persisted sync point. An uncertain write leaves its intent and a restart-visible HOLD. Native Note and direct MCP/file gate save requests are refused for this root until they gain their own durable coordinated admission path.

## Proofs and exclusions

The S3 platform tests reject activation with an open editor session, reject prepared prewrites when no S3 custody route was activated (without consuming the durable HOLD), reject direct File Gate writes with no one-shot token, reject mismatched proposed bytes, accept a matching live+durable ticket once, **reject replay using its still-pending SQLite row**, and preserve NeedsReview for HistoryService failure and a replaced OS-lock inode. This *does not* prove physical rollback resistance, trusted external processes, Windows power-loss durability, native UI crash recovery, rename/trash/copy/create protection or global fail-closed enrolled-root policy. Other File Gate mutation entrypoints still need explicit guard or denial.

**Do not enable automatic S3 overwrite, delete, baseline advancement, GC or user-facing auto-sync on this proof.** The next P0 is durable participant enrollment across Core restarts and complete File Gate mutator coverage.
