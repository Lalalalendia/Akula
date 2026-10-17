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

## Normal history I/O alias preflight (04g-3b-3)

Normal `HistoryService::saved`, `name`, `page`, `version`, `moved`,
`trashed` and `restored` now validate their selected
`<base>/.aquilum/history/...` directories **before** reading,
writing or changing cached location state. The preflight requires all existing
directory components from the trusted selected base to be ordinary
directories; symlinks, Windows NTFS junctions/reparse directories,
`..` components and paths outside the selected history root are rejected.
Trash history is checked against its own `.trash` base.

Checks occur under the existing `PROCESS_HISTORY_FILES` mutex,
without acquiring the File Gate mutex. This is a *static alias guard*, not a
pinned OS file handle, cross-process lock or prevention of external TOCTOU
directory swaps. Low-level `history::store` functions remain callable
outside this guarded service; they are **not** approved as a sync mutation API.
A guard failure logs a refusal, but legacy `bool`, `Option` and `()`
history signatures cannot yet report the full error to the caller.
Dedicated Unix symlink and Windows junction regressions exercise version
read/write denial and nested write/move denial.

## Correct acknowledgement of history persistence (04g-3b-5)

File Gate commits the primary note before attempting to record history.
`HistoryService::try_saved` now returns a typed `Result<bool, HistorySaveFailure>`:
`Ok(true)` means a persisted history change, `Ok(false)` means no change,
and `Err` means history recording failed, **not** that the already-written
primary note failed. Failures emit `note-history-save-failed` instead of
`note-history-changed`. The native host logs this separately and refreshes
an open history panel when the failed operation was only partly applied.

`written_hash` is updated only after the planned history mutation succeeds,
and `last_edit` only after the version-file change succeeds. An unsuccessful
first baseline write retains the original bytes **in memory** for the next
save attempt, which would otherwise see only the newly committed primary note.
Version removal now propagates directory enumeration and unlink errors.

Baseline+version+name persistence and version+name deletion are still
**nontransactional**: a partial mutation is reported as an error, never falsely
acknowledged as complete. A crash can discard an in-memory pending baseline;
interprocess locking, durable recovery and OS-pinned operations remain future
work. These changes do NOT authorize sync local writes.

## Tests and limits

- A real FS two-service regression holds the global history mutex while the second service attempts to persist a new history snapshot. Bytes remain untouched until release.
- A second regression holds the same mutex during a real legacy-history migration and verifies no file moves until release.
- Three-OS Sync pinned-read matrix and required Main PR gate validate history tests, alias preflight, native Rust and visual fidelity.
- This mutex serializes **one OS process only**, globally across unrelated vaults, and an unbounded retention pass may stall HistoryService read/write UI calls. Evaluate per-root partitioning and bounded maintenance with latency measurements.
- The six `history::store` mutation entrypoints (`adopt_quantum_history`, `remove_expired`, `write_version`, `update_version_text`, `remove_version_and_names`, `write_name`) are now `pub(super)`: code outside `history` cannot call them, and compile-fail doctests enforce the public boundary. They remain callable **inside** the `history` module and still require the service interlock and alias checks; this is not an OS filesystem capability. Public read-only store helpers still accept caller paths and are **not** a trusted sync access API. Other direct writers, `.trash` purge/retention, external processes and symlink/junction path swaps remain outside the history mutex. Full history transactional durability, OS handle-relative mutation, crash recovery and cross-process locking remain necessary before enabling remote mutation or local sync apply.

Roadmap: https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11
