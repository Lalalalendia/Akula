# ADR-SYNC-014 — External, physical-root-scoped OS lock inspection

Status: **SYNC-04g custody candidate / non-authorizing diagnostic** (2026-10-18). NOT production lock custody, write authorization or safe sync publication.

## Motivation

PR #161 demonstrated on Windows, Linux and macOS that a file under the user-editable vault can be renamed while another process holds an exclusive advisory lock on its old OS object. A second file at the original path can then carry another independently acquired exclusive OS lock, even with identical UUID and BLAKE3 bytes. Rechecking the pathname only detects substitutions that have *already happened*: it does not protect future directory entries.

In addition, local SQLite workspace UUIDs can differ for two Core processes registered to the same physical vault. The external lock must be located by the same **local physical root identity** in both Core instances, not a per-Core UI UUID.

## New deliberately limited capability

- Private `sync/filegate_lock_custody.rs` provides a **read-only, existing-object preflight** for a host-supplied external custody *candidate*. No directory, lock file or record is created, rewritten, renamed, deleted or auto-repaired.
- Candidate path must point to an existing directory named `vault-<volume-hex>-<file-id-hex>`, where the tuple comes from the registered, pinned OS root handle. This filename is a deterministic **LOCAL scoping hint**, not a globally stable or authenticated VaultId. Drive relocation, inode recycling and mounts require independent persistent attestation.
- The candidate directory must not be inside, equal to, or an ancestor of **any** registered Core workspace. Unreadable or missing registered workspace paths cause refusal rather than being ignored. The candidate root itself is opened and pinned with the established no-follow / physical file-ID checks.
- On Linux/macOS, the candidate directory must be owned by the effective user and have mode `0700`; the already-opened lock file must be owned by that user and mode `0600`. Both checks use metadata from **opened OS handles**, not merely filename stat. This is useful against accidental group/world exposure; it does not prove arbitrary POSIX/macOS ACL absence or prevent a hostile same-UID writer.
- Windows explicitly returns `WindowsAclUnverified`. NT handle identity and refusal of direct reparse points are checked, but no DACL/owner policy has yet been verified. **This observation MUST NOT be treated as a permission check**.
- A Core must already register the selected workspace in its SQLite registry. Its root identity and registry are checked before and **while** the external OS lock is held. The handle is dropped before returning a private snapshot. The result has no write or history methods and grants no durable claim.
- Distinct Core SQLite databases that register the same physical workspace and use the same scoped external directory observe the same OS lock object despite different local workspace UUIDs.

## Native tests

The `host_admission_external_scope_` regressions run in existing Linux/macOS/Windows registered-Core native matrix:

1. Two independent Core instances have different SQLite workspace UUIDs but observe the exact same physical root, external directory and lock-file identities; neither creates a file inside the vault.
2. Missing lock does not bootstrap; wrong scoped directory name and vault-nested scope fail closed; bytes of the existing external lock remain identical.
3. Registry revocation causes refusal without touching the external lock.
4. On Unix, group/world-readable or writable directories/files fail; mode is restored and inspection becomes possible again. The effective owner is validated against `geteuid`.

The established two-process `oslock_` tests from #145/#150/#153/#157/#161 remain in the CI matrix; no production File Gate integration occurs here.

## Unresolved release barriers

1. **Host root custody not authenticated.** An arbitrary path supplied by a malicious same-user process can pass owner/mode checks. Parent ancestry can contain aliases/reparse points; `0700`/owner does not guarantee Windows DACL semantics or strong namespace immutability. An app-only path inside another ordinary writable directory is not a security boundary against its owner.
2. **Physical identity not a permanent VaultId.** Tuple `(volume, file-id)` scopes a currently pinned root on one machine only. Recycled IDs, copied vaults, network mounts and cross-device sync require an independently persisted, authenticated root↔VaultId binding. The preflight neither creates nor verifies such a mapping.
3. **No permanent anchor bootstrap or ACL hardening yet.** The existing `.akula-filegate-oslock-probe` is only a diagnostic object. Privileged or same-user directory writers could still replace it after a check. A robust protocol needs a validated platform-specific security boundary and a single agreed local custody-root location across cooperating processes. A Windows no-DELETE-share handle alone is insufficient for cross-platform guarantees.
4. **Not an active writer guard.** `Core::file_mutation_guard_for_write`, all `HistoryService` write methods, sync local publishing and native editor buffers are unchanged. An observation releases its OS lock **before** returning. No bilateral PREPARED/PUBLISHED/VERIFIED durable receipts, cross-process Core lock ordering, kill/fault matrix or safe recovery exists.

The next P0 slice must choose and validate a real custody-root location outside user-editable roots, Windows DACL/owner checks, anti-alias/ancestor policy, stable VaultId attestation and common writer lock lifetime. Only afterward may a guarded data-write integration be considered; managed cross-vault moves, local sync overwrite/delete, automatic HOLD clear and accepted-baseline commits stay prohibited.

See [ADR-SYNC-013](adr-0013-lock-namespace-custody.md), [ADR-SYNC-012](adr-0012-exclusive-lock-bootstrap-probe.md), [Tela sync plan](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11).