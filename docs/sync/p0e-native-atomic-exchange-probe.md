# SYNC-APPLY-P0e native swap lab (test only)

This module only compiles under `cfg(test)`. It cannot be called from the production editor, S3 transport, File Gate or VaultPort. It exchanges file names ONLY in temporary test fixtures using a pinned parent descriptor. Linux uses `renameat2(RENAME_EXCHANGE)`; macOS uses `renameatx_np(RENAME_SWAP)`. Windows fails closed and does not use ReplaceFileW.

## Native exchange is not an identity CAS

The swap is atomic as a name operation, but does NOT atomically compare either file's identity with a prior value. The probe verifies both old target and staged name against their expected inode and exact bytes immediately before swap. Afterwards it verifies the old/new identities and contents at their exchanged names, and syncs the parent directory. These checks still do not close a malicious namespace race between the last check and the syscall.

The initial probe retained a stage descriptor but rechecked only the destination pathname before swap. A controlled stage-name substitution could therefore publish a foreign file before the postcheck returned `Ambiguous`. The correction also validates the stage pathname, catching a different inode even when its bytes are identical, and catching in-place changes to the stage bytes. Regression cases preserve the displaced stage as evidence rather than silently deleting it. Any syscall failure after entering exchange remains conservatively `Ambiguous`; there is no fallback to plain rename or automatic rollback.

An independent local Linux syscall reproduction demonstrated both cases before and after the additional check. That reproduction was not execution or compilation of the Rust crate. Exact-head repository CI is the acceptance source for the Rust implementation.

## Panic evidence versus actual process termination

The original `catch_unwind` test is retained as a panic/unwind test. It does NOT prove SIGKILL behavior, power-loss persistence or the existence of a durable journal fence.

A separate parent/child test now uses the real V7 `SqliteSyncJournal`. The parent seeds a synthetic previously confirmed revision and a Staged download, closes its SQLite connections and launches exactly one child-test entrypoint. The child commits `record_existing_replace_attempt_once` and enters the existing native swap probe. It signals a bounded readiness barrier at each of three points:

1. `before_exchange`: staged bytes have been synced; the original target has not been exchanged.
2. `after_exchange`: names have been exchanged, before postflight verification and parent-directory sync.
3. `after_parent_sync`: native postflight checks and parent-directory sync returned successfully, but no native receipt or baseline commit exists.

At each barrier the parent calls `Child::kill`, waits/reaps the child and asserts Unix signal 9. A 30-second readiness timeout and a kill/wait drop guard prevent a silently skipped helper or an orphan child. This is not merely an injected error or normal exit.

After the kill, the parent opens new descriptors and a new journal connection. It checks exact old/new bytes and the retained original inode, the existing V7 claim, unchanged previous baseline and `Staged` phase. Reissuing the same claim must fail with `ExistingReplaceAttemptAlreadyRecorded`; generic `mark_applied` must still return `LocalDownloadNeedsNativeReceipt`, and recovery must require inspection. Successful evidence emits `AKULA_NATIVE_EXCHANGE_SIGKILL` with the reached boundary, real signal and rejection result.

## Limits and admission

These are test-only operations on disposable directories. The baseline is seeded using the real journal API but synthetic provider data; this is not a verified remote S3 transfer. No Core File Gate/HistoryService is held across this native swap. The current test journal and original-byte fixture are not a production protected-original store or independently trusted installation source. Process termination does not simulate machine power loss, filesystem remount, disk failure or malicious same-user mutation.

Even a successful `after_parent_sync` case grants no publication receipt, no retry and no confirmed Sync baseline. Production still needs one admitted host/root lifetime across File Gate reservation, V7 claim, protected original outside the vault, platform-native replacement, history persistence and independent post-write readback. Windows remains explicitly unsupported for this native swap experiment. Keep the PR Draft until exact-head Linux/macOS/Windows and Application checks finish; do not enable automatic S3 apply.

References: Linux rename documentation at https://man7.org/linux/man-pages/man2/renameat2.2.html; Rust Child::kill/wait semantics at https://doc.rust-lang.org/std/process/struct.Child.html; original macOS reference https://newosxbook.com/src.php?file=%2Fbsd%2Fman%2Fman2%2Frename.2&tree=xnu.
