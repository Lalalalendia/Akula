# ADR-SYNC-027 — Different appdata can bypass one Core's participant HOLD

Status: **negative two-process falsifier only**. This is a release blocker, not an admission protocol or a new mutation permission. All test operations use disposable temporary workspaces; no S3 endpoint is contacted.

## Current safety failure

ADR-SYNC-022 / #235 stores a full-synced `guarded_participants` marker in the first Core's appdata `documents.sqlite3`. Subsequent Cores using THAT SAME appdata correctly deny all ordinary File Gate mutations. But `file_mutation_guard_for_write_mode` checks only its own Core's participant registry and history recovery markers. There is no mandatory root-scoped lookup of another profile's S3 participant or shared OS lock before permitting an ordinary writer.

A fresh profile B pointing at the **same physical registered vault root** can therefore take the unguarded write/create path while A remains held. Process-local publication mutexes (#242/#245/#247) don't synchronize independent OS processes, and the authorized guarded S3 writer's external custody lock is not acquired or checked by ordinary B.

## The executable negative probe

`oslock_falsifier_alternate_appdata_bypasses_enrolled_core_hold` under `core/src/files/gate/interprocess_probe.rs`:

1. Create temporary root, appdata A, appdata B and an independent external lock scope.
2. Core A records a guarded participant with a shared VaultId, and real File Gate operations through A now reject.
3. Parent pins and exclusively holds a live kernel lock under the external test custody scope.
4. Spawn a **real second OS process** running the existing Rust child test entrypoint, but with only appdata B and the same vault root.
5. Assert the child CAN currently overwrite A's original note and create another note, without consulting A's enrollment or external lock. Confirm A remains on HOLD. This is a finding, not a pass for release.

The test is intentionally written to assert the **currently observed unsafe fallback**. When a production fix is introduced, INVERT these expectations so that B rejects both operations, and retain the negative/future-positive proof on Linux, macOS and Windows.

## Required positive authority design

A valid solution needs a shared, OS-protected and physically pinned custody authority that an independent appdata profile cannot silently bypass, including after appdata deletion/reinstallation. A local workspace UUID does not serve as a cross-profile VaultId. A pathname or BLAKE3 checksum is not authenticated custody. Do not create or clear a missing anchor on ordinary write.

Cooperating-process guarantee requires a single stable root-scoped anchor with verified identity, explicit enrollment/recovery state, kernel OS lock through primary + history, and fail-closed admission from **all ordinary and guarded** File Gate/HistoryService/native writers. Test appdata B/A start orders, crash during write, Windows junction and Unix symlink/inode swaps, lock replacement, and running one process while the other restarts. Hostile same-UID actors require an OS security principal/broker beyond owner-writable files (ADR-SYNC-013).

**Blocked until completed:** public/automatic S3 publishing, overwrite/delete, baseline advancement, GC and automatic HOLD clearance. This falsifier is not an endorsement of current behavior.
