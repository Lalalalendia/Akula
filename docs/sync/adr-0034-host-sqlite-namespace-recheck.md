# ADR-SYNC-034 — Host SQLite name and WAL/SHM negative custody checks (P0-B5a)

**Status:** narrow fail-closed, cooperative namespace protection. NOT an installer trust anchor, proof against same-user hostile processes, or permission for S3 mutation.

## Threat motivating this slice

[PR #274](https://github.com/Lalalalendia/Akula/pull/274) and [PR #278](https://github.com/Lalalalendia/Akula/pull/278) prove the native `Workspace::open` missing-registry HOLD and a separate-process positive `unenrolled` write to a real common ledger. Neither pins SQLite's *physical database name* through a note publication epoch or rejects symlink/reparse aliases for adjacent WAL/SHM files. A process may otherwise continue on a formerly opened SQLite inode while the compiler-pinned path is replaced by a separate valid, empty registry, risking two independent writer locks.

## Narrow implementation

- In `host_ledger_identity.rs`, retain a read-only, opened handle to the real SQLite database and the parent directory, using the already available pinned root/Windows file-ID routines. Do not create or repair any ledger file.
- Reject non-regular main database/WAL/SHM paths, symlink or Windows reparse leaves, and multiply-linked SQLite files. WAL and SHM may legitimately be absent until SQLite creates them.
- At `open_existing` and `provision_new`, capture physical IDs independently of SQLite's connection, and recheck the main pathname and immediate parent after initial schema verification.
- At `BEGIN IMMEDIATE`, record current WAL and SHM identities and recheck the main path **before** File Gate's primary+HistoryService closure. Recheck all three identities before and after commit; errors after the closure are ambiguous/NeedsReview at the Core boundary. Apply the same checks to installer registration and ordinary host registry inspection. Never silently retry or declare the root unenrolled after an identity mismatch.
- No new writer, CAS exception, token, global lock, file deletion, sqlite recovery/bootstrap, or S3 mutation. Existing ordinary local builds and the `--features installed` release pipeline retain their behavior.

## Acceptance

The existing [host-custody CI](../../.github/workflows/sync-host-custody.yml) runs Core registry and native negative tests on Linux, macOS and Windows. The [native installed positive CI](../../.github/workflows/sync-native-positive-host.yml) proves 2 native processes can still create/write a note, observe both full history versions, reject stale-CAS, and cold-reopen on each OS.

New registry falsifiers verify: (1) an existing DB handle refuses admission while its path has a second hard link; (2) already-substituted `-wal`/`-shm` symlinks refuse before SQLite can use a redirected sidecar (Unix); and (3) a different valid-empty DB swapped under the same filename **inside** an active transaction fails with no apparent successful epoch, keeping old bytes for inspection (Unix). Only the test fixture manipulates its own temporary host namespace.

## Remaining mandatory P0

These are **point-in-time rechecks**, not a secure namespace against an attacker who can swap directory entries between checks. A new cold-open into a replacement *valid* empty database is still possible until an installer or privileged broker supplies an immutable/pinned expected ledger ID. Neither owner/ACL guarantees nor physical identity of every ancestor, WAL and SHM from live kernel handles across their full lifetime are proven. Same-user malicious writers, crash/ambiguous ACK, durable PREPARED/PUBLISHED/VERIFIED receipt, correctly enrolled S3 custody and installed two-device end-to-end are not admitted. Do not enable S3 overwrite/delete, GC, Sync Beta, or treat this as a production release gate.
