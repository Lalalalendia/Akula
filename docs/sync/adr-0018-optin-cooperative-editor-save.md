# ADR-SYNC-018 — Explicit, cooperative editor save through the registered S3 custody lock

Status: experiment; opt-in internal Core activation only. **No public S3 synchronization switch or hostile-writer protection.**

## Problem

ADR-SYNC-017 and PR #210 prove an OS lock held for a single existing-note CAS write and its HistoryService snapshot. The normal editor uses `DocumentHub → HubIo::write → files::gate::write`, which did not participate in that lock, so it could race the protected S3 additive-create writer in another cooperating Core.

## This slice

- Add an **explicit runtime-only** `activate_registered_s3_editor` host operation. It is not called by desktop UI or Core::open and creates neither a new lock nor a remote identity. It requires an existing registered workspace, stored S3 profile with distinct local workspace UUID and remote VaultId, a separate appdata custody SQLite receipt, and the exact pre-existing outside-vault OS lock.
- Keep the binding immutable for an already armed root within a Core. Do not accept overlapping nested armed roots. After Core restart, the host must run a new explicit validation/activation; **ordinary writes are not safely protected just because old databases exist**. A persistent fail-closed participant policy remains P0.
- Once armed, `files::gate::write` for a matching path routes the real editor's CAS update into the existing `save_existing_note_under_custody`. The original OS lock stays held through the File Gate mutex and the HistoryService attempt; checks of the S3 profile and custody SQLite are repeated on every call.
- A missing `expected_hash`, changed S3 profile or lock inode, busy OS lock, invalid path, unsupported file type or corrupted custody receipt refuses **without ordinary-write fallback**. After entering the primary writer, any failed File Gate, HistoryService or postflight check becomes `NeedsReview` and is delivered as `DiskError::Other { code: "needs_review" }`, not a confirmed save.
- Editor conflict/truncation preservation uses the independent unguarded File Gate create path. It is intentionally **refused** for armed workspaces rather than writing outside the shared OS lock. The unsaved editor content remains in its session; safe guarded conflict-copy creation is subsequent work.
- Native S3-feature tests call the actual `HubIo::write`, check history persistence, injected history failure classification, copy denial and physical lock-name replacement refusal. The Windows/macOS S3 platform workflow and Linux feature CI run them.

## Limits and next action

This experiment protects only the explicit matching `gate::write` path in **participating** Core instances. Other writers — editor protective-copy create, direct HistoryService operations, rename, trash, restore, retention, plugin commands and independent processes that do not opt in — still bypass this route. Same-user processes may forge appdata and replace lock namespaces. There is no broker-owned custody, participant enrollment enforcement, durable activation marker across restarts, cross-vault transfer admission, sync local overwrite/delete, baseline advance, HOLD clearance, UI or sync scheduler.

Before production activation, all mutation paths within a participating workspace must either hold the same lock through history or explicitly refuse; a durable fail-closed participant policy must prevent a restarted Core from writing unguarded. Then test two cooperating editor processes, forced kill at primary/history boundaries, and review latencies.

See ADR-SYNC-016/017.
