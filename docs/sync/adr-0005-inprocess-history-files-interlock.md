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

## History relocation acknowledgement and retry quarantine (04g-3b-6)

`HistoryService::try_moved/try_trashed/try_restored` report typed errors
without silently updating `State.notes` or `Note.dir` when version-folder
relocation fails. The primary note operation already completed under File
Gate; a failure therefore emits `note-history-move-failed` (from, to,
action, error, possibly-partial) instead of misreporting a failed note rename
or silently claiming history arrived.

A process-wide quarantine (under `PROCESS_HISTORY_FILES`, shared by
all cooperating Core instances) records BOTH source and destination note
identities after a failed file transfer, including preflight denial. History
version writes, labels and reads refuse blocked locations. Retry of the
**same** source/destination pair under the history lock merges remaining
version files; an unrelated transfer cannot clear that quarantine. History
maintenance is deferred while any unresolved pair remains, to avoid
deleting potentially split versions. No callbacks or Core File Gate
acquisition occur while holding the history mutex.

`move_folder` still permits incremental merges into an already existing
target, so filesystem operations are NOT atomic: failure can leave versions
in both trees. Never assume rollback, and never automatically delete one
side. Quarantine is **in-process only** and is lost on restart; crash-durable
recovery manifests, an explicit repair UI, cross-process coordination and
handle-relative operations are future requirements. Blocking an unrelated
vault's retention while another vault has unresolved history is conservative
and needs later per-root partitioning.

## Persistent version-transfer intent and inventory admission (04g-3b-7a)

Before changing **history version files** for a rename, trash or restore,
the Core now records a per-workspace one-in-flight intent at
`<vault>/.aquilum/history-moves/pending-v1.json`. It contains the selected
operation, source and destination identities, history paths and the exact
union of relative filenames and BLAKE3 content fingerprints from both
history trees. The manifest is written using `create_new`, flushed with
`File::sync_all`, and its containing directory is fsynced on Unix.
Directories created for the journal are also synced on Unix.

A process restart no longer loses the recovery signal: each subsequent
history read, version write, label write and maintenance pass checks for
an existing or **unreadable/malformed** intent under the process history
mutex and refuses normal history operations. A fresh `HistoryService`
with different app-data cannot silently fork history while an intent
remains in the shared vault. While a transfer is unfinished, the Core
does NOT silently remove a version or announce that its history moved.
The host continues to receive the separate
`note-history-move-failed` event, independently of the already-moved
primary note.

The **same operation and same source/destination pair** may be retried.
Before retry and after the move, the service compares the union of file
digests against the recorded manifest: every original file must exist on
exactly one side and retain its exact bytes. The intent is cleared only
when the source contains no recorded files and the destination matches
the complete inventory. Even if the source directory has disappeared,
missing or modified versions cause a fail-closed error; they are never
treated as a successful transfer. Name collisions and unknown files are
rejected. Case-only renames on case-insensitive filesystems use a single
tree inventory. Intent entries with symlinked/junction/reparse ancestors
are rejected, and corrupted manifests are **not auto-deleted**.

**Remaining restrictions:** the marker is a conservative
cooperating-process checkpoint, **not** a complete cross-process locking
protocol. In particular, another process can still modify a path after
its pathname preflight, separate processes may register different sets
of workspaces, and an untrusted process can modify the marker. On
Windows the marker file is flushed, but parent directory fsync/power-loss
durability is not certified; neither platform has proved full power-loss
durability for the underlying history directory renames. A pending marker
blocks all history activity in a registered Core's known workspaces
conservatively until its transfer is safely retried. An explicit repair
screen, operator-guided response to corrupt inventory, trusted
interprocess admission, and OS-handle-relative mutation remain P0.
**Local sync write/delete authority is still disabled.**

## Cross-vault fail-closed admission before primary rename (04g-3b-7b-0)

PR #121 writes a **source-workspace** intent before moving version files. A
Core that has registered **only the destination workspace** cannot see that
source intent after a process restart. Simply writing two independently
created markers is not an atomic transaction: an interruption between their
creation can leave only one visible.

Until a bilateral admission mechanism can prove that **both participating
workspaces** have safely recorded the operation **before** the primary note
moves, File Gate now refuses to rename or move a managed path across
registered workspace boundaries. This includes folder moves and moves between
a registered workspace and an unregistered destination. The refusal occurs
**before** filesystem mutation, link rewrites, history calls and
`NotesRelocated` events, under the ordinary in-process File Gate.
It deliberately does not acquire `PROCESS_HISTORY_FILES` inside File Gate.

The admission uses both lexical and canonically resolved paths to identify
the selected registered vault. Ambiguous nested registrations or alias
disagreements fail closed. A rename that stays inside one unambiguous vault
continues to work as before. Paths wholly outside registered roots remain
subject to the existing ordinary File Gate policy, not an inferred sync
write permission.

Direct `HistoryService::try_moved` calls also refuse cross-vault history
relocation before generating a source-only intent. They cannot silently
convert this into an apparently durable cross-vault history transfer.

**Intentional temporary limitation:** moving a document or folder to another
vault is unavailable even if that item currently has no version history.
This is a product restriction, not an acceptable permanent replacement for
cross-vault moves. The next milestone must establish a bilateral,
crash-recoverable admission phase **before primary mutation**, with two-Core
source-only/destination-only tests and deterministic process termination
between each state transition. An explicit human repair path for any
unfinished transfer is also required.

This change **does not** provide an OS-level interprocess lock, protected
open directory descriptors, power-loss proof, or authority for sync writes.

## Tests and limits

- A real FS two-service regression holds the global history mutex while the second service attempts to persist a new history snapshot. Bytes remain untouched until release.
- A second regression holds the same mutex during a real legacy-history migration and verifies no file moves until release.
- Three-OS Sync pinned-read matrix and required Main PR gate validate history tests, alias preflight, native Rust and visual fidelity.
- This mutex serializes **one OS process only**, globally across unrelated vaults, and an unbounded retention pass may stall HistoryService read/write UI calls. Evaluate per-root partitioning and bounded maintenance with latency measurements.
- The six `history::store` mutation entrypoints (`adopt_quantum_history`, `remove_expired`, `write_version`, `update_version_text`, `remove_version_and_names`, `write_name`) are now `pub(super)`: code outside `history` cannot call them, and compile-fail doctests enforce the public boundary. They remain callable **inside** the `history` module and still require the service interlock and alias checks; this is not an OS filesystem capability. Public read-only store helpers still accept caller paths and are **not** a trusted sync access API. Other direct writers, `.trash` purge/retention, external processes and symlink/junction path swaps remain outside the history mutex. Full history transactional durability, OS handle-relative mutation, crash recovery and cross-process locking remain necessary before enabling remote mutation or local sync apply.

Roadmap: https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11
