# SYNC-APPLY-P0e-WIN — isolated Windows ReplaceFileW falsifier

**Status:** test-only. No production mutation entrypoint, File Gate permission,
S3 executor, native publication receipt, or baseline promotion. The source is
compiled only by `#[cfg(all(test, windows))]` from `sync/mod.rs`. CI uses
disposable `tempfile::TempDir` directories on the Windows runner.

## Why not reuse the Unix name exchange?

Windows does not implement Linux `renameat2(RENAME_EXCHANGE)` or XNU
`renameatx_np(RENAME_SWAP)`. Its `ReplaceFileW` has different semantics:
the replacement receives the destination pathname, the original may be moved
to a named backup, and the replacement inherits some metadata/streams from
the old file. Microsoft explicitly documents distinct failure states:
`ERROR_UNABLE_TO_REMOVE_REPLACED` (1175),
`ERROR_UNABLE_TO_MOVE_REPLACEMENT` (1176), and
`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` (1177). An unsuccessful call can still
change names or attributes. **Treat every entered-call failure as ambiguous.**

Reference: https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-replacefilew

## Exact test boundary

- Open the disposable root with the existing NT handle-based no-reparse
  helper, recording volume + file identity.
- Read the old regular file relative to the root handle, requiring a single
  hard link, exact old bytes and the expected file identity.
- Create the unique stage relative to the pinned root with `FILE_CREATE`,
  write bounded new bytes (maximum 32 MiB), flush the stage via
  `File::sync_all`, and record its file identity. Never overwrite an
  existing stage or backup name.
- Immediately before `ReplaceFileW`, recheck the root, old target, stage
  identity/bytes, and that the backup pathname remains unused. Inject
  deterministic same-bytes target and stage substitutions to verify refusal.
- Call `ReplaceFileW(old, stage, backup, 0, NULL, NULL)` only inside a
  disposable test. Preserve the return code as diagnostic data; failure is
  always `Ambiguous` with no retry or cleanup.
- On success, reopen both names relative to the original root handle and
  verify that the new bytes have the old pathname with the staged file ID,
  and the old bytes are intact under the unique backup name with the old ID.
  The original stage name must be absent.
- Exercise a Windows `FILE_SHARE_DELETE` denial and an injected post-call
  lost acknowledgement. Neither grants a remote baseline.

## Explicit security limits

The operation takes *pathnames*, not the pinned NT file handles. Thus
preflight is not an identity compare-and-swap and same-user writers may
race it after the final check. The backup is in the test root; an actual
production protected original must be retained outside the user vault and
tied to a registered host/custody epoch. ACLs, inherited alternate data
streams, EFS/compression and metadata inheritance must be reviewed before
production. A write-through ReplaceFileW flag is not supported.

The staged handle is synced, but this test does **not** establish
crash/power-loss durability of directory metadata on Windows or a
platform-independent parent-directory fsync. `catch_unwind` is only a
process-local panic test; no Windows TerminateProcess/power-cut proof is
claimed here. Successful native postflight is **not** a V7 journal receipt
and cannot authorize `mark_applied` or `ConfirmedBaseline`.

## Next gates

1. Exact-head Windows-native CI and relevant Main PR/Rust checks.
2. Independent process termination at before/after syscall and
   after-readback/lost-ACK boundaries, with real V7 claim persisted and
   reopened across processes.
3. Link registered host/root custody, original-byte protection,
   existing guarded File Gate and HistoryService and typed postflight
   receipt to the exact immutable V7 operation.
4. Separately prove Windows durability and error-state recovery before
   considering a production executable. No silent fallbacks or retries.

This PR deliberately does none of steps 2–4.
