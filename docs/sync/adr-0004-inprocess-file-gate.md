# ADR-SYNC-004 — In-process File Gate serialization

Status: **candidate, not sync-write authorization** (2026-10-17).

## Problem

The read-only registered-root preimage capture from PR #69 holds a
DocumentHub closed-editor reservation, but an ordinary File Gate mutation
can originate from a host command *without* an open editor session. That
writer could race SQLite preimage preservation. A live preimage's root and
source IDs alone do not exclude every other writer.

## Narrow implementation

All Core instances in a process now reference the **same process-static**, fail-closed
Mutex shared by File Gate entrypoints and
RegisteredReadAdmission::preserve_closed_original. The mutex protects the
filesystem portion of each managed operation, including calls from distinct
Core instances accessing the same workspace:
- note and binary creation, note write with expected hash, copy;
- directory/file rename and link rewrites as one operation;
- move-to-trash and each restore-from-trash mutation;
- entire registered-root preimage capture, SQLite persistence and second
  pinned read, while the DocumentHub closed-editor reservation is retained.

Callbacks, editor reconciliation, history bookkeeping and search/index
notification are explicitly performed **after** release of the Gate mutex.
These must never be invoked while holding the mutation lock, since editor
autosaves can hold the sessions mutex while entering gate::write.

The DocumentHub reservation temporarily acquires sessions -> reservation
locks and releases those mutexes before the Core Gate lock is acquired.
The immutable reservation guard is retained until after the capture finishes.
Gate writes never hold the File Gate mutex when acquiring DocumentHub
sessions locks; the integration tests assert no deadlock on concurrent
create, write, copy, rename, trash and restore.

A poisoned mutex denies future managed writes. This is intentionally
fail-closed rather than silently falling back to unsafe unlocked writes.

## Important limits

- This is **one OS process, across all its Core instances**, not an
  interprocess lock or an OS-backed opened-file mutation capability. External
  processes, direct platform-specific raw file handles and direct calls to
  low-level filesystem helpers can still change data. In particular,
  history/trash cleanup and service-specific maintenance paths require a
  separate coverage audit before claiming complete in-process exclusion.
- Gate writes are still conventional path-based operations; locking them
  does NOT make them root-handle-relative, protected, journaled or safe
  against symlink/junction races. The sync engine gains NO write method.
- A long SQLite backup can now delay editor writes in **every Core instance
  in the same process**, including unrelated workspaces. This is an explicitly
  conservative security tradeoff; per-workspace keyed locks and latency
  profiling should be evaluated before enabling the protected-write executor.
- Locks make existing local writes sequential, not globally atomic across
  all affected files. Restore batches remain independent operations.
- A durable bound preimage is not a PreservationReceipt. The SQLite store's
  generic unbound method is not a trusted host identity bridge.

## Required validation

Test on Linux, macOS and Windows with real Core and filesystem operations:
while a test holds the shared mutex, each normal Gate operation must remain
blocked and complete after release. A second independent Core instance with
another app-data root but the same workspace must also be blocked. The
preimage capture must hold that same mutex while retaining root identity,
editor and hash checks.
Run existing sync pinned-read, SQLite v2 and full Rust suites.

## Next security gates

1. Audit every low-level workspace writer and maintenance path; route
   writers that may touch protected sync files through the interlock.
2. Add explicit OS-handle-relative trusted mutation authority and target
   identity rechecks, rather than adapting the path-based editor writer.
3. Acquire a host-wide lock under a documented no-deadlock lock order,
   independently durable protective copy and verified staged bytes.
4. Require atomic replacement, fsync, readback and journal recovery on
   Windows/macOS/Linux with path-swap and power-loss fault injection.
5. Never enable sync local writes, S3 executor or Sync UI until all are green.
