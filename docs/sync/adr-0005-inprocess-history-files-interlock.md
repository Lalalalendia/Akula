# ADR-SYNC-005 — Process-wide history store/maintenance interlock

Status: incremental in-process durability guard (2026-10-17), **not** authorization for sync writes.

## Confirmed race

Every `HistoryService` previously serialized its own `State` with a private mutex, but native Settings retention invoked `history::store::{adopt_quantum_history,remove_expired}` directly. A second `Core` could write, rename, read or label the same history directory concurrently with cleanup or migration. PR #80 added symlink/junction *preflight*, not history scheduling.

## Coordination

- A single `PROCESS_HISTORY_FILES: Mutex<()>` protects **cooperating HistoryService version-store** reads/writes across all `Core` instances within one process.
- `saved`, `name`, `page`, `version`, `moved`, `trashed`, `restored` acquire the process mutex **before** their per-service state mutex. On process mutex poison, version-store access is refused rather than performed unlocked; the current history API cannot propagate an error for all operations, so it may report an empty view or no recorded change, and logs the refusal.
- Native Settings calls `HistoryService::maintain_retention`, which acquires only the process history mutex while migrating `.quantum` histories (including the trash root) and deleting expired `.aquilum/history` versions; SQLite UI-state cleanup stays outside both locks.
- `HistoryService::disk_before` uses only the private state mutex to inspect an original note, **never** the process history-files mutex. File Gate operations may call `disk_before` while their own global File Gate is held, so this avoids waiting for a long maintenance scan while holding the File Gate.

## Required lock-order boundary

`History-files -> HistoryService state` is valid; `File Gate -> HistoryService state` is used for `disk_before` only. **Never** acquire File Gate, emit callbacks or reconcile DocumentHub while holding the process history-files mutex. File Gate operations release their lock before `saved/moved/trashed/restored` history bookkeeping. The history files mutex does not grant, verify or change sync write permissions.

## Tests and limits

- A real FS two-service regression holds the global history mutex while the second service attempts to persist a new history snapshot. Bytes remain untouched until release.
- A second regression holds the same mutex during a real legacy-history migration and verifies no file moves until release.
- Three-OS Sync pinned-read matrix and required Main PR gate validate history tests, alias preflight, native Rust and visual fidelity.
- This mutex serializes **one OS process only**, globally across unrelated vaults, and an unbounded retention pass may stall HistoryService read/write UI calls. Evaluate per-root partitioning and bounded maintenance with latency measurements.
- Public low-level `history::store` helpers and other direct writers, `.trash` purge/retention, external processes and symlink/junction path swaps are **not** covered by this mutex. Full history transactional durability, OS handle-relative mutation, crash recovery and cross-process locking remain necessary before enabling remote mutation or local sync apply.

Roadmap: https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11
