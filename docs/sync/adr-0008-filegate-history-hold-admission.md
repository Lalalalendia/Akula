# ADR-SYNC-008 — Ordinary File Gate refuses pending history recovery before primary mutation

Status: 04g-3b-7c-b1, safety-only, no sync write authority (2026-10-17).

## Gap fixed
PR #137 added a per-vault source/destination-visible read-only HOLD signal for HistoryService. It previously blocked version-history reads/writes/retention but ordinary Core File Gate could still commit the **primary** note and then discover history journaling was blocked. This could leave a user-visible primary edit without a matching history version. The same mismatch applies to a stale or corrupted legacy `pending-v1.json`.

## Decision
Use a separate `Core::file_mutation_guard_for_write` for mutating entrypoints, preserving the existing read-only `file_mutation_guard` needed for forensic observation, registered-root scanning and recovery inspection.

1. Acquire the existing **process-wide** File Gate mutex.
2. Read the complete registered workspace list from SQLite. Unreadable registry fails closed; never treat errors as an empty list.
3. For **every** registered workspace, check both history relocation markers using the existing read-only `history::recovery::pending` API, without taking `PROCESS_HISTORY_FILES`. A present, malformed, unreadable or unsafe (symlink/junction) marker refuses mutation.
4. Only when no HOLD is observed may the ordinary mutation run. The check is held inside the same in-process File Gate critical section as the operation. All formerly mutating ordinary Gate entrypoints and the guarded MCP trash purge use this path: write/create/copy/binary/create-folder/template, move/rename, trash/restore, retention cleanup.
5. History labeling remains governed by HistoryService's existing HOLD check. Read-only forensic inspection must not be disabled by this change.

The scan intentionally covers all roots registered in a **single Core**, matching the current conservative HistoryService quarantine behavior. A different Core that has registered **only an unrelated vault** remains free to operate in that vault. Per-root partitioning is a follow-up, not an excuse for allowing primary mutation while HistoryService refuses versions.

## Evidence
- `history_cross_vault_filegate_hold_denies_primary_mutations_and_keeps_versions` verifies primary bytes and recorded history names unchanged after all ordinary File Gate mutation attempts and demonstrates unrelated-root-only Core progress.
- `history_cross_vault_filegate_corrupt_hold_cannot_allow_primary_write_after_restart` verifies fail-closed corruption and restart behavior.
- Required Rust workspace, sync-engine and Linux/macOS/Windows pinned-read matrix, plus Native frame fidelity.

## Non-guarantees / next work
The check is still **pathname-based and advisory** relative to external processes: a foreign process can swap or remove a marker after inspection; it does not install an interprocess lock or defend a fully hostile filesystem. Windows directory metadata power-loss durability is not established. This PR does not create any marker, clear a one-shot claim, authorize a managed cross-vault move, authorize sync writes/overwrite/deletion, or declare a recovered baseline.

The added synchronous SQLite + filesystem check on every mutating File Gate entrypoint has nonzero latency. Measure real edit/save and retained-marker performance; optimize via a proven invalidation-safe cache only after preserving fail-closed semantics.

Next: canonical real cross-process lock/handle identity in **both** vaults, bilateral durable PREPARED, crash tests before primary mutation, operator review. See [ADR-SYNC-007](adr-0007-bilateral-cross-vault-recovery.md).
