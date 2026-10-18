# ADR-SYNC-029 — Shared host serialization epoch, P0-B1

**Status:** experimental cross-process negative admission primitive, stacked on P0-A PR #252. NOT attached to normal File Gate or HistoryService, and NOT authorization to publish remotely.

## Why a read-before-write registry check is unsafe

An unenrolled Core B can inspect the installed host database, see no record, and begin ordinary primary-file publication. Concurrent Core A could then insert a protected S3 participant before B writes history. The path lookup and File Gate operation must be serialized with enrollment under a single **cross-process** synchronization object. A process-local Rust mutex/RwLock cannot do this.

## Implementation

The installed `SharedHostCustodyRegistry` now exposes a narrow `with_unenrolled_epoch(root_path, pinned_root_id, operation)` primitive:

- Require a real, non-symlink/non-junction directory candidate, canonicalize its spelling, and require a separately supplied physical root identity.
- Start SQLite `BEGIN IMMEDIATE` in the **same host database** whose `enroll` method also takes `BEGIN IMMEDIATE`. This is an exclusive SQLite writer transaction using OS file locks on the same stable physical host database.
- While holding the transaction, refuse a matching enrollment or conflicting root identity and do not call `operation`. If unregistered, run `operation` while holding the lock, and release only after it returns. A later enrollment waits for completion; a previous enrollment forces refusal.
- A cross-process Rust test starts a real child OS process with another appdata context. The parent retains the epoch across a real File Gate primary write and HistoryService; the child cannot finish enrollment while it is held. After release the child inserts the enrollment and a repeat epoch denies before its mutation closure runs.

**This is not an atomic commit across SQLite and primary/history files.** If the operation crashes, persistent recovery still matters; SQLite rollback cannot undo filesystem changes. The production caller must include the full primary+history lifetime and its own CAS, durable intent, physical root lock and recovery checks.

## Hard boundaries

- **Not yet wired:** unrestricted `Core::open`, ordinary native File Gate, direct HistoryService and external OS writers still bypass this primitive. PR #251 remains a passing *unsafe* falsifier until its assertions are inverted after real integration.
- The installed host path must be identical and host-trusted in every cooperating process; an optional caller-selected path allows downgrade and is not sufficient.
- SQLite WAL lock semantics must be verified on each supported local filesystem. Do not trust remote/network mounts, symlinked database ancestors, replaced DB inode, hostile same-user namespace writers, or a host process choosing another database. A privileged installer/broker must enforce a stable, protected directory and physical identity.
- A root lookup must be pinned against symlink/junction and moved-root races; the current canonicalization is negative lookup evidence, not a safe file capability.
- This primitive is not included in the normal editor hot path until P0-B2 policy coverage and latency testing exist.

## Next gates

P0-B2 binds one installer-provisioned host database to **every** Core on startup, and routes every ordinary File Gate + direct HistoryService operation through the shared epoch while preserving File Gate/History lock order and keeping UI callbacks outside it. P0-C replaces the old two-process #251 success-on-bypass assertion with a **deny** assertion on Windows/macOS/Linux, and covers create, overwrite, rename, trash, restore, direct history, kill/restart and changed lock identity. P0-D then narrows process-wide HOLD to per-root without allowing downgrade.

Until all are accepted, no automatic S3 overwrite/delete, baseline promotion, GC or automatic HOLD removal.
