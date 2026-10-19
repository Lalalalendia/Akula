# P2P-BLOB-11a — FD-relative Linux custody (isolated Staya research)

**Status:** stacked experimental Draft atop P2P-BLOB-10c #357. No change to Akula main, user documents, S3, production sync, File Gate or pinned Roha/noq forks.

## Threat model and exact guarantee

Prior P2P-BLOB-09/10 proof code checked pathnames and re-opened files by strings between sequential scans. A symlink, bind-mounted directory, renamed file or mount swap between steps could turn a previously verified path into a different inode. Merely comparing file content hashes does not establish that two sequential pathname opens are referencing the same physical object. Source: Linux openat2(2) resolve semantics.

**11a solves a bounded subproblem only.** An existing private, separately mounted Staya volume is pinned by an open descriptor. All descendant opens use pinned rustix 1.1.5 `openat2`, with `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_XDEV`, `O_CLOEXEC | O_NOFOLLOW | O_RDONLY` and `O_DIRECTORY` for directory entries. Unknown kernels / ENOSYS / EAGAIN / malformed pathnames **HOLD** with no pathname fallback.

A `PinnedFsStore` owns independent descriptors for the store, `data`, `temp` and `blobs.db`; explicit re-resolution from the held mount checks inode identities before and after proof validation. Owned `.data` and `.obao4` are each opened *once* from the pinned `data` directory, then checked for regular-file type, single link, exact device and bounded length; all reads and full BLAKE3/Bao verification use those SAME descriptors. A copied store with a renamed/replaced `data` directory must HOLD even if contents are identical.

The pre-existing durable ledger remains exclusively owned, journal reservation root and expected length are checked, and a full P2P-10a joined audit precedes this independent check. A bounded pinned `blobs.db` digest is measured before and after. During the pinned Bao pass, a **redb::ReadOnlyDatabase** handle is opened through a self-owned `/proc/self/fd/N` descriptor path and holds a transaction open. If procfs or redb read-only access is unavailable, HOLD. This pins the redb backing inode and uses the pinned database's read-only locking behavior, but NOT a filesystem-wide lock against arbitrary writers.

## Physical CI

The same private 64MiB Linux ext4 image from 10b/10c must:
1. Reopen the actual pinned `iroh-blobs 0.103.1` FsStore with 8MiB+17B external data and exact 32KiB external Bao, and its independent fsynced journal.
2. Perform a second full BLAKE3 and authenticated complete pre-order Bao proof using ONLY the pinned data/outboard descriptors and journal-trusted root. Check the pinned read-only redb descriptor and unchanged bound paths.
3. Refuse a test-only symlink to an otherwise valid store and refuse `../` escape.
4. On an independent disposable store copy, open safe directory descriptors, rename `data` away, replace it with a new directory and require `ChangedDirectoryBinding`. Do NOT modify the original FsStore or journal.
5. Actually unmount/remount the ext4 volume and re-check the original data, DB, outboard and journal with newly acquired FDs.
6. Preserve every previous Roha/noq, QUIC unauthorized-PUSH, bounded ENOSPC, SIGKILL, redb, Bao and docs/rustfmt gate.

## What remains HOLD

**This does NOT prove exclusive control of a writer, a single atomic filesystem/database/journal snapshot, or that an administrator cannot change an open inode's bytes.** redb cooperative locking alone does not stop a non-redb process from overwriting `.data` or `.obao4`. The previous P2P-10a audit is still sequential/path-based. Inode identity and timestamps can detect many changes but cannot exclude an adversarial write-and-restore race. `fd_relative_no_symlink_no_xdev=true` means only the new descriptor-based validation, not the entire historical audit chain.

Next distinct barrier is P2P-BLOB-11b: independently enforce and test a stopped/quiescent FsStore writer (including forked owner-handle inheritance), and fd-anchor the redb inventory itself across the combined audit. Then partial `.sizes4/.bitfield`, interrupted physical writes, Windows/Android bounds, signed VaultId/DeviceId authorization and File Gate integration remain separately blocked.

All return types end with `independent_writer_exclusion_unproven=true`, `partial_bitfields_unchecked=true`, `atomic_filesystem_snapshot=false`, `file_gate_ready=false`. Never merge into main or clear HOLD based solely on laboratory passing tests.
