# RELEASE-SIGNED-STAGING-01 — signed candidate → fsynced stage → rollback (HOLD)

Status: **stacked Draft integration, not an activated updater**. Depends on [#483](https://github.com/Lalalalendia/Akula/pull/483) and uses the signed-manifest type already merged in [#488](https://github.com/Lalalalendia/Akula/pull/488), plus the GUI ACK merged in [#498](https://github.com/Lalalendia/Akula/pull/498).

## Exact new boundary

1. A private `SignedCandidate` exists only after an Akula manifest signature has been verified with the selected public key and all version/platform/package/sha256 constraints have passed. There is no public unchecked constructor.
2. The new Linux-only `updater/linux_signed_stage.rs` bridges `SignedCandidate` into the **existing** SIGKILL-tested `linux_transaction::Gate`. It acquires and retains the real OS `flock`, writes the candidate to a same-directory generation through the original `Gate::prepare` (candidate and receipt `sync_all`, stage and parent directory sync).
3. The original sealed `SignedCandidate` reopens the freshly written file with Linux `O_NOFOLLOW`, compares its device/inode and single-link identity against the current directory entry, streams a SHA-256 digest in 64 KiB blocks, checks actual length and unchanged inode, and refuses any digest or path mismatch. `PendingSignedStage` owns the gate lock and the signed metadata; it is **not** a publish permit.
4. The production `stage_signed_candidate()` always returns HOLD: `TRANSACTION_ACTIVATION_APPROVED=false`, `release::channel_ready()=false` and `PUBLIC_KEY=""`. The legacy `apply()` also continues to return HOLD before installation.
5. A **cfg(test)-only** positive integration uses a real temporary Minisign keypair and signed manifest generated in the required Application CI, confirms the signed exact payload, stages it on Linux and rehashes the on-disk inode, then demonstrates same-sized corruption refusal and symlink refusal while preserving Aquilum data.
6. After resetting the disposable staged test file, that same test uses the original transaction methods to publish fake v2 bytes into the old v1 path, simulate a missing GUI ACK, move v2 bytes to `failed.AppImage`, and restore the old v1 bytes. The old Akula binary and Aquilum witness must be byte-for-byte unchanged after rollback. The test-only publication function is not built into production binaries.

## CI acceptance

- `Akula Linux AppImage rollback lab` continues to compile exactly the native transaction module with `rustc -D warnings` and pass 11 tests, including three real cross-process SIGKILL recovery boundaries.
- `Application checks` executes the exact native `ephemeral_minisign_fixture_accepts_exact_bytes` Rust test on the signed synthetic corpus, now requiring both markers in actual test logs:
  - `AKULA_SIGNED_LINUX_STAGE_PASS real_minisign=1 fsync=1 disk_sha256=1 tamper_hold=1 aquilum_intact=1`
  - `AKULA_SIGNED_LINUX_ROLLBACK_PASS signed_candidate=1 v1_restored=1 v2_preserved_as_failed=1`
- Linux AppImage / DEB-RPM, Windows Beta, macOS installer (when applicable), Native frame and Main PR gate must pass on the **exact head** before the PR can be considered for merge. Do not reinterpret skipped, queued, or cancelled checks as success.

## What this does NOT prove

This is a signed **synthetic-byte** publication and simulated missing-ACK file transaction, not a real packaged Akula v1→v2 GUI update. It does not start the replacement GUI, cryptographically attest the GUI PID or current executable inode, run a powered-off rebooter, persist a rollback-resistant version floor, or establish an Akula publisher signing root. The future supervisor must authorize the exact installed path and bind a real ACK through a verified process/pidfd before any destructive rename. Current staging and launch entrypoints remain denied even if this lab passes.

No installed Akula/Aquilum files were modified by the tests; only disposable CI fixture paths are written. Public release, automatic update, S3/Sync file mutation and macOS updater activation remain HOLD.

Canonical specification: [AKULA-MASTER-INDEX-01](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b).
