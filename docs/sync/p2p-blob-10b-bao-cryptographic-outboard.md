# P2P-BLOB-10b — cryptographic Bao outboard validation (Linux laboratory)

**Status: stacked experimental draft over [P2P-BLOB-10a #347](https://github.com/Lalalalendia/Akula/pull/347), then #343/#337/#327/#321/#314/#301.** No Akula main, S3, File Gate, user data or Roha/noq fork mutation.

## Security gap and pinned upstream semantics

P2P-10a confirms a quiet isolated Linux FsStore, typed redb metadata, trusted BLAKE3 content and durable ledger agree. This does NOT prove the accompanying Bao outboard tree is safe for authenticated range transfer: a corrupted internal parent hash could be sent to another peer even though the source file BLAKE3 itself is correct.

Pinned iroh-blobs 0.103.1 uses a **16KiB chunk-group (BlockSize::from_chunk_log(4)) PRE-ORDER outboard**. Its default options inline small outboards in redb; a genuine 8MiB+17 byte file should produce a physical hash-addressed .obao4 alongside .data. Our experiment covers only this *complete owned outboard* case, leaving redb-inline proofs as the next task.

Upstream [iroh-blobs bao_file.rs](https://github.com/n0-computer/iroh-blobs/blob/v0.103.1/src/store/fs/bao_file.rs); [bao-tree 0.16.1 validated ranges docs](https://docs.rs/bao-tree/latest/bao_tree/).

## Implementation

- Exact existing Cargo.lock version **bao-tree 0.16.1** added as a direct dependency ONLY in the isolated Staya Linux lab crate. Pinned Roha/noq, iroh-blobs and other dependencies unchanged.
- New bao_outboard_audit.rs: uses the already-held P2P-07 journal owner lock, P2P-08 mounted finite Linux volume and the completed P2P-10a joined journal/DB/physical-file observation. Requires one verified journal reservation matching the exact BLAKE3 and size. Restricts to 16KiB to 256MiB complete objects, and at most 4MiB of external Bao proof bytes.
- Refuse symlink/hardlink/wrong-device/overlarge/unavailable external data or .obao4. Construct the native BaoTree size/group and PreOrderOutboard root from the **journal-trusted BLAKE3**, not a remote claim.
- Call the Bao library's **encode_ranges_validated** for ALL chunk ranges, with output discarded in an IO sink. Any Bao parent or leaf hash mismatch/read error leads to HOLD, without network writes or repaired files.
- Positive result is OwnedBaoProofEvidenceOnly: the full external proof is validated, but **file_gate_ready=false**, **inlined_bao_unchecked=true**, **partial_bitfields_unchecked=true**, **atomic_filesystem_snapshot=false**. Opening paths by name remains insufficient against malicious privileged TOCTOU.

## Real physical CI acceptance

The dedicated 64MiB ext4 loopback GitHub lab, following previous external/inline/joint/ENOSPC tests, must:

1. Build a **real iroh-blobs 0.103.1 FsStore** with 8MiB+17B immutable test content, confirm there is a nonempty external .obao4 and close/flush the store.
2. Save a new private durable journal containing verified content hash and length, check full BLAKE3 and prove all authentic Bao parents/chunks with bao-tree and the root from that ledger.
3. Copy the store into a **separate disposable test-only directory**, corrupt a single byte of that COPY's outboard; required result HOLD even when data, journal and redb still appear consistent. Remove the COPY's outboard and require HOLD. Original DB, original proof bytes and journal are unchanged.
4. Physically unmount/remount the same finite ext4 image and repeat read-only proof verification of original FsStore and original persistent journal.
5. All preceding pinned QUIC/deny-PUSH, physical ENOSPC, no-mount, redb inline, joint audit, lock handoff and SIGKILL regressions remain green.

## Explicit non-goals

The majority of smaller/mid-size blobs use Bao INLINE redb proof records: still not verified by this task. Partial bitfield and sizes4 metadata are not authenticated or reconciled. Neither this nor the P2P-10a joined check is a single race-free fd-relative filesystem/DB snapshot. No Windows/Android implementation, signed VaultId/DeviceId grants, production File Gate, host quota adapter or actual user-file P2P sync is enabled. Keep stacked **draft and unmerged** after green laboratory CI. Next subtask is a single pinned redb transaction with full inline Bao verification and then partial-receipt/bitfield testing.
