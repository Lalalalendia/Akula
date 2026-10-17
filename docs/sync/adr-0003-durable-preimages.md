# ADR-SYNC-003 — Durable preimages in app-data SQLite (Stage 04g-3b-0)

Status: **prototype, not a local write authority** (2026-10-17).

## What this implements

The sync Core contains a crate-private, inert SqliteProtectionStore. A trusted
caller may persist verified bytes under an operation ID and portable relative
path in a host-selected, PRIVATE app-data database outside the workspace.
The table stores the original bytes, BLAKE3 hash and original relative path.
The SQLite connection requests WAL and synchronous=FULL; each backup is
committed in an IMMEDIATE transaction, then read back and verified.

The same operation ID, path, content hash and exact bytes is idempotent.
Reuse of an ID for different content/path refuses without overwriting the
original copy. Bad hashes, oversized content (more than 32 MiB), corrupted
stored bytes, invalid portable paths and future schema versions fail closed.
A successfully committed original survives a process restart. Empty content
is a valid original and is protected too.

## Deliberate non-goals

- This store is **not yet wired into Core**. It accepts already-verified bytes;
  it does not read a live host workspace, reserve editors or prove filesystem
  identity and cannot independently authenticate the remote planner.
- SQLite durability protects the preimage subject to SQLite VFS, OS and storage
  flush guarantees. It is not an independent disk backup or disaster-recovery
  copy. Database path, ownership and access controls MUST be selected and
  validated by the trusted host. Never point it at the user workspace.
- There is NO PreservationReceipt constructor or VaultPort implementation.
  A record's presence is **not** proof that a live source file was safely
  preserved under a current OS File Gate lock. A later adapter must bind the
  exact live file identity, expected BLAKE3, editor buffers and operation.
- No S3 download may apply a staged object locally; no replace, rename,
  delete, baseline commit, credential or Sync UI is enabled by this stage.
- Backup retention, encryption at rest, private app-data path authorization
  and secure restore UI need their own tracked requirements before production.

## Next implementation gate (04g-3b-1)

1. Under the host's exclusive File Gate and editor reservation, use a pinned
   root-relative descriptor and an opened regular file identity to capture a
   bounded original, verify its expected hash and reject path/hardlink races.
2. Durably store the verified preimage in the protection database in the
   host-selected app-data location, reopening to verify the committed copy.
3. Only after 1–2 have succeeded, have the host create a nonforgeable,
   single-operation PreservationReceipt referencing the live file identity
   and the verified stored original. Recheck again before any destructive act.
4. Implement the atomic OS-relative replacement and readback/journal gate as
   another separate PR with crash and filesystem swap/race injection tests.
5. Require Linux, macOS and Windows matrix tests, including fault injection,
   editor aliases, unsubmitted buffers, NTFS reparse, path swaps, disk failure
   and readback mismatch. Unsupported OS semantics must refuse mutation.

Cross-reference: design PR #1 and read-only root admission PR #58.
