# ADR-SYNC-015 — Negative Windows owner/DACL audit and lock-anchor ancestor checks

Status: **SYNC-04g custody negative-gate / no write authority** (2026-10-18). This is NOT immutable custody, filesystem-agnostic OS lock, authenticated VaultId, or permission for File Gate / HistoryService writes.

## Why this follows #168

The [external lock custody preflight](adr-0014-external-lock-custody-scope.md) scoped one existing OS lock outside all registered vaults. On Unix it rejected weak owner/mode settings but on Windows it returned `WindowsAclUnverified`. Also the direct leaf/root pinning did not reject parent-directory symlinks or junctions, and the workspace-roots list used in the overlap test was captured only **once** before the OS lock.

## New limited checks

1. **Windows pinned-HANDLE security descriptor:** The Windows lock opener requests `READ_CONTROL` in addition to existing minimal rights. New private `filegate_windows_acl.rs` calls `GetSecurityInfo` for owner and DACL of each *opened directory/lock HANDLE*; nothing is resolved through a second global pathname for ACL collection. Owner must match the current process token's user SID or its validated default owner SID (TokenOwner, which may be an Administrators group under an elevated token). A missing/null DACL, malformed ACE, unknown ACE layout or read failure causes refusal.
2. **Windows allowed ACE audit:** The auditor iterates the actual DACL with `GetAclInformation` / `GetAce`. It considers effective ACEs (inherit-only ACEs do not grant access to the current object, and the child lock is independently audited). It refuses any effective allow ACE granting create, write, delete, WRITE_DAC, WRITE_OWNER, GENERIC_ALL or GENERIC_WRITE to a SID other than the token's user, LocalSystem or BUILTIN\Administrators. Deny ACEs cannot add access. Unsupported callback/object ACEs are **not** silently trusted. The audit executes both before and while the external OS lock is held. Result `WindowsOwnerDaclRestricted` conveys *negative ACL evidence*, not validated effective access.
3. **Ancestor alias denial:** Each component of the candidate external custody directory must be an existing real directory; direct symlinks, Unix ancestor symlinks, Windows junctions/reparse points and non-absolute paths refuse. Path ancestry is rechecked while the lock is held. This is still a **pathname-based preflight susceptible to same-user races**, not an immutable handle chain.
4. **Fresh registry overlap check:** The inspector now obtains registered roots twice using the Core SQLite accessor, once before opening the candidate and again while the OS lock is held. This closes the stale-snapshot diagnostic gap if a *new* registered workspace overlaps the selected custody directory during inspection.
5. **No authority expansion:** The lock file and its parent must exist already; the inspector never creates/deletes/updates lock or vault content. Every handle is dropped before returning. Core File Gate, HistoryService and sync local publication are unchanged.

## Native acceptance

- Windows: two independent Core databases sharing one external lock, baseline owner/DACL inspection, explicit Everyone/full-control grant to **directory** and **lock file** rejected without data modification; all existing native `oslock_` split-lock tests pass.
- Unix/macOS: a symlinked ancestor of an otherwise valid external lock denies; canonical absolute scope still works and requires modes 0700/0600 with effective owner.
- All platforms: a mock registration source adds a registered workspace overlapping the external custody scope *after* the first snapshot; the second read under OS lock refuses, no recovery/autocreation.
- Full Rust workspace and sync-engine, Native frame fidelity, three-platform pinned-read, protected-originals, Docs, S3/MinIO, Main PR gate on exact head.

## Remaining P0 / excluded threat models

- **Malicious same-UID or elevated actors:** On Windows a process holding the same user token, SYSTEM or Administrators can modify/delete the same objects. On Unix the owner can edit mode or rename entries. This policy reduces accidental/broad grants but does not make lock names unreplaceable.
- **No complete Windows effective-access / ACL proof:** Inherited object/callback ACEs are conservatively refused. The check does not model token group privileges, ownership takeovers, integrity levels, privileges, DACL changes after check, junction changes after recheck or reparse ancestors within a hostile racing namespace. Non-NTFS/network filesystem semantics require independent tests. OS error returns must fail closed.
- **No persistent authenticated custody root:** The caller still supplies a candidate host pathname named by an observed physical volume/file ID. The recent [S3 workspace-VaultId binding](https://github.com/Lalalalendia/Akula/pull/167) is not by itself an attestation of this local filesystem root or its Windows ACL. Root relocation, ID recycling, copying a vault and multiple users need explicit policy and an immutable/broker-held directory handle chain.
- **No writer lock lifetime / cross-process deadlock proof:** This is a read-only diagnostic preflight; it releases its lock before return. The protected, process-local File Gate and HistoryService writers still need a shared, stable OS anchor, agreed lock ordering, lifetime guard, bilateral journal receipts and crash matrix. No managed cross-vault move, sync overwrite/delete, automatic HOLD release or confirmed-baseline commit is authorized.

References: [ADR-SYNC-013](adr-0013-lock-namespace-custody.md), [ADR-SYNC-014](adr-0014-external-lock-custody-scope.md), [canonical Tela roadmap](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11).
