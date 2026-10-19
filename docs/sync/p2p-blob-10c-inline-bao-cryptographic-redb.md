# P2P-BLOB-10c — cryptographically authenticated redb-inline Bao (isolated Linux lab)

**Status:** stacked Draft experiment on [P2P-BLOB-10b #354](https://github.com/Lalalalendia/Akula/pull/354) → #347 → #343. Not application Sync Core, S3, File Gate, Android, Windows or user data.

## Pinned upstream invariants

- `iroh-blobs = 0.103.1` and `redb = 4.4.0`, `bao-tree = 0.16.1` are already pinned in the isolated Staya crate.
- Default `FsStore` puts data >16KiB in a physical `<hash>.data`, but **pre-order outboard bytes <=16KiB inside `inline-outboard-0`**.
- `Complete::OutboardLocation::NotNeeded` requires `BaoTree::outboard_size() == 0`; `Inline` requires *non-zero exact-sized raw pre-order bytes*, no prefix or suffix.
- Cross-table integrity requires one read transaction for `blobs-0` (typed complete entry) and `inline-outboard-0` (raw proof). Do not interpret malformed or partial EntryState as complete.

## Implemented proof gate

- Existing P2P-10a joined quiescent audit first verifies the same exclusively-held fsynced journal, external .data full BLAKE3 and typed redb manifest.
- P2P-10c performs an additional strict **single-redb-read-transaction** scan: for every complete blob check the exact outboard layout, and for every inline-outboard row require a verified matching journal entry.
- For external .data plus inline outboard: retain borrowed raw proof bytes while the read-only redb transaction is open. Require canonical preorder Bao footprint and bounded total proof bytes, check regular same-volume single-link source file with expected length/device/inode, and run `bao-tree::io::sync::encode_ranges_validated` over all chunks with a **journal-trusted BLAKE3 root**. Output goes only to an IO sink.
- Missing, extra, unknown, mis-sized, tampered, incompatible or unexpected inline-outboard records return HOLD. The existing standalone P2P-09b/10a entrypoints are unchanged: their existing "Bao pending" evidence contract remains intact.
- Non-default `inline DATA + inline Bao` pairings are not silently accepted: they remain HOLD until a separate validated data-and-proof path is implemented.
- Public return is **InlineBaoEvidenceOnly**, not a write permit. It always reports `file_gate_ready=false`, `atomic_filesystem_snapshot=false` and `partial_bitfields_unchecked=true`.

## Dedicated bounded-ext4 tests

After 10b external Bao test, on the SAME private 64MiB ext4:
1. Generate a real pinned FsStore with **128KiB + 37B** external data and a real non-empty **redb-inline** outboard. Confirm no corresponding physical `.obao4` exists.
2. Fsync independent journal with the exact BLAKE3, then validate the whole tree.
3. Create a complete disposable FsStore copy and mutate **only copied** redb records: altered parent, appended extra bytes, missing proof row. Every case must HOLD; the original source database, payload and journal bytes stay untouched.
4. Present an independently valid wrong hash/ledger and require HOLD. Check a genuine tiny `OutboardLocation::NotNeeded` baseline with a verified inlined data blob.
5. Perform a real ext4 `umount`/`mount` and revalidate the SAME original journal, data and proof.
6. Preserve existing 10b external proof, direct pinned QUIC/deny-PUSH, real VFS ENOSPC, SIGKILL/lock, docs/fmt tests on exact SHA.

## Explicit security limitations

**NOT an atomic filesystem + redb + journal snapshot.** The two phases are serial observations under a cooperative already-shut-down FsStore. Path-based file discovery remains subject to privileged TOCTOU/mount swaps and requires future fd-relative custody and true exclusive writer control. P2P partial `.sizes4/.bitfield`, interrupted writes, orphaned locks/fork inheritance, signed peer grants, quotas for Android/Windows and File Gate remain HOLD. This lab never grants production P2P publication, protected writes or quota release.
