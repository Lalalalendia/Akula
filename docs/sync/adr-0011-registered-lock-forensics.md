# ADR-SYNC-011 — Registry-bound existing-lock forensics

Status: SYNC-04g-3b-7c-b4, **dormant, production-compiled internal diagnostic only** (17 October 2026). Not File Gate write admission.

## Problem

PR #145 established test-only two-process OS lock mutual exclusion; PR #150 pinned the lock file to an OS directory descriptor/HANDLE, validating file/volume IDs, no-follow semantics and single-link objects. These test fixtures could select any temporary directory, not a workspace registered to a real Akula Core. Neither a stable pathname nor a successful OS advisory lock proves write authority.

## New internal contract

- `sync/filegate_lock_probe.rs` becomes a private, production-compiled helper. The lock remains named `.akula-filegate-oslock-probe` and must already exist. No bootstrap, migration, lock creation/unlink or user-facing entrypoint is added.
- `RegisteredReadAdmission::inspect_existing_filegate_lock()` can be invoked only after opening the workspace with the existing SQLite registry + pinned directory identity checks. It validates the exact Core's registered workspace UUID/path and physical root ID, opens the pre-existing lock relative to that descriptor, and temporarily obtains its exclusive OS advisory lock.
- While that lock is held, it REVALIDATES the SQLite registry and original root file ID again. If the root, registered mapping, lock identity or OS exclusivity cannot be verified, inspection refuses. The lock handle is dropped before returning.
- Its private `RegisteredLockObservation` contains the workspace UUID and OS root/lock ID tuples, solely for diagnosis. **It is a snapshot, not a durable claim, lock token, lease, preparatory acknowledgement, or mutation receipt.** It cannot be used to bypass Core File Gate or history HOLD.
- The normal File Gate, HistoryService, native editor host and S3 sync executor have **no callers** of this method and do not change behavior. Windows root-relative NT FILE_OPEN support is compiled privately, never exposed to network or plugin code.

## Native regression evidence required

- Linux, macOS and Windows: registered Core + pre-existing lock returns matching physical identity, no lock file content change; a different Core with a separate unregistered SQLite registry is refused even though the OS root and lock exist; after explicit registration, physical IDs agree but workspace UUIDs differ.
- Missing lock remains missing (no implicit creation); hardlink alias refuses; after registered workspace revocation a previous admission refuses; after renaming the root and creating a replacement at the registered pathname the previously opened admission refuses and both original/replacement bytes remain.
- Existing PR #150 `oslock_` suite remains required for two-process contention, partial-lock release, root substitution and symlink/hardlink alias. Run exact-head full Rust workspace, sync-engine, native UI, Docs and three-platform pinned-read CI before merge.

## Release barriers NOT satisfied

- No secure, durable bootstrap of a **permanent lock object** whose custody/ACL prevents malicious name replacement. A foreign process can unlink or substitute a lock pathname after the method's final check. Passing this diagnostic must NEVER permit a normal File Gate write.
- SQLite binds workspace UUID to a path; it does NOT persist an independently attested OS directory ID across restarts. Root replacement after restart and unreliable/network filesystems need a separate explicit policy.
- No cross-process real File Gate + HistoryService lock-order integration, two-party durable PREPARED/PUBLISHED/VERIFIED commit, Windows directory durability proof or operator recovery. Ordinary managed cross-vault relocation, local sync overwrite/delete and confirmed-baseline commits remain prohibited.

Next P0: trusted registered-root anchor provisioning/custody and cross-process writer proof without namespace split-lock; then bilateral crash matrix with persistent, human-reviewable recovery.

References: [ADR-SYNC-007](adr-0007-bilateral-cross-vault-recovery.md), [ADR-SYNC-009](adr-0009-two-process-lock-probe.md), [ADR-SYNC-010](adr-0010-pinned-lock-identity-probe.md), [canonical Tela roadmap](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11).