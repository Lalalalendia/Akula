# P2P-BLOB-09a — fail-closed external-file inventory vs durable journal

**Scope: Linux research-only; stacked on [P2P-BLOB-08 #327](https://github.com/Lalalalendia/Akula/pull/327), then P2P-BLOB-07 #321, #314 and #301.** No production Akula sync, File Gate, S3, user workspace, Roha/noq fork or native desktop change.

## Problem

A green capacity-bound ext4 test does not establish that the bytes remaining on disk after restart correspond to objects authorized and reserved by the P2P-BLOB-07 durable journal. A stale unreferenced hash, leftover interrupted temp, symlink, or corrupted complete payload must result in **HOLD**. An automatic deletion is too dangerous: journal and FsStore can contain data from a concurrent or interrupted operation.

In pinned [iroh-blobs 0.103.1 PathOptions](https://github.com/n0-computer/iroh-blobs/blob/v0.103.1/src/store/fs/options.rs), the external layout includes data/<hash>.data, .obao4, .sizes4, .bitfield; temp/<UUID>.temp is used for atomic moves. The root also contains redb blobs.db. The store requires temp and data on the same filesystem.

**Crucial limitation:** small blob payloads and their metadata may be **inlined in blobs.db**. A pathname scan cannot prove that the database contains no unknown object. Therefore this stage returns strictly **InventoryEvidenceOnly**, *not* a clean approval and *not* a File Gate permission. Verified journal entries with no external .data are placed into HOLD until an independently trusted read-only database parser is added; they are not silently treated as missing bytes or freed space.

## Implementation

- A small read-only inventory API on the exclusive P2P-07 DurableLedger exposes exact persisted (hash, expected size, verified flag) views. It does not alter ledger generations, records or byte counters.
- P2P-08 StagingVolumeGuard exposes the pinned mount root (without changing its fail-closed checks). The new Linux-only reconcile_inventory module only scans one immediate child FsStore root on that mounted device while the journal lock is held and the store is quiescent.
- Strictly recognizes the top-level FsStore layout: blobs.db, data and temp only. It rejects missing/zero-size DB, unknown names, symlinks, hardlinked files, nested directories, wrong device, malformed UTF8 and bounded enumeration overflow.
- Temp must be empty. Any residual temporary entry is HOLD, without attempted recovery/deletion. A data file is permitted only if its name has exactly a 64-character lowercase hex BLAKE3 name with suffix data, obao4, sizes4, or bitfield, AND that hash exists in the journal.
- An external data file larger than its recorded expected size is HOLD. For journal records previously marked verified, a real external data file is required (otherwise HOLD, including valid but not-yet-audited inline cases). Its length and streamed full BLAKE3 are checked, not merely filename or file length. A corruption is HOLD.
- All reads have test-focused bounds (at most 4096 directory entries and 256MiB of separately hashed verified external data). The mount identity is checked before and after scanning. The scanner never invokes a writeable FsStore API and does not modify the DB or ledger.
- **No automatic cleanup, quota release, File Gate publish or user file writes.** For now positive results still carry database_contents_uninspected=true and file_gate_ready=false by contract.

## Verification

A real disposable 64MiB ext4 loopback image in the read-only GitHub workflow hosts actual patched iroh-blobs 0.103.1 FsStore and its P2P-07 reservation ledger. After a verified blob is persisted and FsStore gracefully shut down, a lab-only test performs the positive external inventory then injects, one at a time, an orphan external hash, an in-place wrong-byte/same-length payload, a residual temp file, an unexpected top-level path and a temporarily displaced blobs.db. Every condition must cause HOLD. The test restores only its own disposable changes, checks that the journal snapshot bytes did not change during observation, then remounts the entire ext4 image and runs the audit again against the same durable ledger and real FsStore.

The existing Linux ENOSPC, FsStore remount, missing-mount refusal, peer-denial and SIGKILL tests remain mandatory regressions. Exact-head CI must pass before claiming this test was accepted.

## Explicit limitations / next gate

**P2P-BLOB-09a is not full startup reconciliation**. In particular:
- It does not open or verify the redb blobs.db tables or discover inline blobs, tags or in-database unreachable/orphan references. That requires a pinned, independently tested, read-only parser with bounded decoding and an exhaustive comparison to journal references.
- It does not cryptographically check Bao outboards, bitfield state or partial ranges. A verified external .data hash is necessary, not sufficient, to authorize sync. Any incomplete blob stays unpromoted.
- It does not exhaustively account for other directories outside this FsStore root on the bounded volume, nor does it enforce physical quota itself.
- The audit is pathname-based and not yet fd-relative and race-free under a malicious concurrent local writer/remount. The test only runs against an exclusive, shut-down store.
- The test does not simulate OS power loss, torn sectors, Windows/Android mounts, real peer enrollment or File Gate.

Next step P2P-BLOB-09b: read-only redb metadata reconciliation using the pinned table schema, explicit unknown-schema HOLD, plus fd-relative staging custody. Continue with actual kill during ext4-backed partial transfers only when these boundaries remain intact. Leave all PRs **draft**; do not merge this experimental code into main.
