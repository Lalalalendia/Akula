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

- The store now has an internal, read-only **host admission capture** path.
  Its generic `store_preimage` still accepts caller-provided bytes and is
  permanently **unbound**; only the new guarded capture can insert an
  identity-bound row. Neither path authenticates remote planner decisions.
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


## Stage 04g-3b-1a: identity-bound *read-only* capture (2026-10-17)

The original v1 SQLite table remains readable. Version 2 adds an atomic
`bound_preimages` table keyed by the immutable operation ID, with four
fixed-width hexadecimal u64 values: opened workspace-root device/file ID
and opened source-file device/file ID. Existing v1 originals remain **unbound**:
they cannot be promoted to a host-proven copy just because they contain the
same bytes. The v1→v2 schema migration runs in one transaction and retains
the original contents. Reusing an operation ID with a new root/file identity
fails closed, including when the contents have not changed.

`RegisteredReadAdmission::preserve_closed_original` now holds its editor
reservation during the entire operation: it checks host registration and the
root descriptor, opens each portable child under a pinned parent, verifies
a regular-file identity and single hard link, reads up to 32 MiB, verifies
the caller's expected BLAKE3, commits both backup bytes and source/root IDs
atomically in SQLite FULL/WAL, and then **reopens** the same portable path
under the pinned root. A different file identity or different bytes after
the commit returns an error. Even after an error the old original may remain
durably preserved; it must never be silently deleted or interpreted as proof
that replacement is safe. The store path is rejected if it canonically lies
inside the registered workspace. The store's creation path itself is still
host-supplied and does not independently verify ACLs/ownership.

Tests inject root and leaf swaps between first read and DB persistence,
including equal-content inode replacement, editor-open denial, duplicate
operation ID collisions, hard-link refusal, SQLite metadata corruption,
v1 migration and cross-process restart of the database. These are deterministic
race windows and NOT a proof against all concurrent filesystem mutations.

**Still intentionally missing:** exclusive host-wide File Gate, coordination
with every non-sync writer, persistent authorization of the app-data backup
database, durable OS-specific atomic mutation, journal readback/commit gate,
disk power-loss fault testing and a trusted `PreservationReceipt`. A
`BoundProtectedOriginal` is a checked backup *record*, not a capability.
Path/file IDs can be reused by the OS and a third-party process can modify
a file after the final read; therefore no local sync writes are enabled.
The remaining 04g-3b-1/2 sub-stages must solve these before any destructive
operation is possible.
