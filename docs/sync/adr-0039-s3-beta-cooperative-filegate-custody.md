# ADR-SYNC-039 — Cooperative S3 Beta File Gate custody on Windows

Status: **implemented in PR #649, exact-head CI pending; opt-in beta only**.
Date: 2026-10-23.

## Decision

Publish a bounded **new-file-only, same-Windows-user S3 Sync Beta** before requiring a physical two-PC Yandex test. Preserve the existing File Gate: no direct S3-to-vault writes, no overwrite/delete, no confirmed durable baseline/ACK, no UI `Synced` status when a create is unconfirmed.

Normal stable Akula is unchanged. The `s3-pair-beta` feature has a separate Native import/setup/sync action and is not enabled by default.

This ADR is NOT an assertion that ordinary user-profile directories are an installer-owned, immutable namespace. A privileged OS/broker-attested installation source and complete power-loss receipts are still missing and are necessary before claiming hostile same-user resilience or fully automated stable sync.

## Why the prior implementation could never pass the installed A→B test

`native/src/s3_pair_session.rs::start_sync_once` passed **`None`** to `sync_new_files_once`. Core correctly sent the upload but returned `NeedsReview` without running the guarded receiver. Its existing `receive_single_verified_s3_additive_once` demands:

- Registered/pinned local workspace identity, separately verified remote genesis and S3 binding.
- A pre-existing **external** lock-scope directory keyed by the local root's physical `(volume,file-id)`, not by portable `VaultId` or independently generated workspace UUID.
- A real kernel lock on an existing non-reparse lock file held across File Gate create.
- The existing Core `SqliteCustodyIdentityStore` binds **workspace + cloud vault + root physical identity + lock physical identity** immutably, and no baseline/ACK is promoted on an ambiguous result.

Previous ADRs 012–016 and 030 established the non-authorizing diagnostics and the stricter full installed-host model. Replacing File Gate with `std::fs::write`, copying a SQLite file from A to B, or treating a path/name match as OS authority was rejected.

## Grounded OS behavior

1. Windows `CreateFile` share mode can deny rename/delete while a handle is held, but an ordinary path string alone is not a durable identity. See [Microsoft CreateFile](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew) and [symbolic-link effects](https://learn.microsoft.com/en-us/windows/win32/fileio/symbolic-link-effects-on-file-systems-functions).
2. Windows `LockFileEx` provides byte-range/shared-exclusive locks on an opened file, not a global lock on a pathname or an authenticated installation. See [Microsoft LockFileEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex).
3. Windows link/junction/reparse handling requires a no-follow/pinned-handle policy. Akula already has `pinned_windows.rs`, `filegate_lock_probe.rs`, negative owner/DACL checks, and physical-ID comparisons.
4. New per-user configuration gets a stable **OS-known** non-roaming root from [SHGetKnownFolderPath](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shgetknownfolderpath) with [FOLDERID_LocalAppData](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid). Do **not** take it from `LOCALAPPDATA`/user-supplied env, cloud JSON, imported `.akula-sync`, vault, or `--data-dir`. Call `CoInitializeEx` in the worker thread and `CoTaskMemFree` on the returned string.

## Implementation contract (one existing PR, not new micro-PRs)

**Native** `s3_pair_session/custody.rs` resolves the actual Windows known folder. Only a deliberate **Configure** or **Import** may create its per-user base `Akula-S3Pair-FileGate-V1`. A later **SyncNow** may *only* open that base. No ambient credential or path fallbacks.

**Core** `s3_pair_host/custody.rs` validates that the host base is absolute, outside all registered local vaults and independent of private appdata, with no symlink/reparse ancestors. It derives `vault-{volume:016x}-{file:016x}` from `RegisteredReadAdmission`, creates the scope with exclusive directory creation and the existing lock leaf with atomic `create_new`. UNIX test fixtures additionally use 0700/0600; Windows validates inherited owner/DACL through the already-existing pinned-handle audit.

The first user-approved provision binds the **already verified S3 profile** to a fresh immutable local custody record. On every later SyncNow, it opens the *same physical scope and lock*, checks the recorded S3 VaultId and lock identity, and rejects a missing/swap/corrupt record. **Nothing is auto-repaired or re-created during SyncNow**.

Native `start_sync_once` passes the validated **`Some(&scope)`** through the existing host `sync_new_files_once` and into the guarded no-replace receiver. The default installed binary still has no S3 Sync Beta feature.

## Threat model and exclusions

- **In scope:** accidental overwrite/collision; competing cooperative processes with different local SQLite DBs but the same physical root; ordinary restart; symlink/junction substitution already visible before validation; missing or altered lock; remote VaultId mismatch; wrong file identity; unverified remote content; fail-closed status.
- **NOT in scope:** malware running as the same Windows user or administrator, privileged namespace swaps after checking, malicious service impersonation, network filesystem or mount identity changes, NTFS power-loss durability, per-device IAM revocation, update/rename/delete of existing notes, universal host-wide write serialization.
- `%LOCALAPPDATA%` ownership and the Windows negative DACL audit do **not** replace a privileged installer/broker witness. This is a conscious opt-in **cooperating-process beta** boundary; no stable-production / hostile-adversary claim.
- The physical two-installed-Akula private Yandex test remains **post beta artifact publication**, before stable promotion. The automated tests plus a working Windows Native receiver, File Gate and package are **pre-beta gates**.

## Required evidence

- [ ] Core Rust tests: explicit provisioning then cold reopen, immutable root/lock identity, unrelated appdata, missing scope/lock refuses, replaced lock after restart refuses, no auto-repair on SyncNow.
- [ ] Windows native beta feature compiles, OS known-folder COM call works in a background worker, no S3 secret enters AppConfig or UI state.
- [ ] Existing isolated MinIO additive A↔B proves exact-byte transfer and BLAKE3 with the true File Gate receiver. A successful Linux Core test cannot substitute for installed Windows.
- [ ] Required PR #649 exact-head Application/architecture/security, Windows beta candidate + installer, and MinIO CI PASS before publishing the **installable S3 Sync Beta**.
- [ ] On that published beta, user-led Windows A→Yandex S3→B and B→A, wrong password, offline, import replay, collision and cold restart must PASS before any stable promotion.

Related: [S3 one-pass runbook](S3-ONEPASS-USER-RELEASE-20261023.md), [ADR-014](adr-0014-external-lock-custody-scope.md), [ADR-015](adr-0015-windows-acl-and-ancestor-custody.md), [ADR-016](adr-0016-persistent-custody-identity.md), [ADR-030](adr-0030-host-core-ordinary-filegate.md).
