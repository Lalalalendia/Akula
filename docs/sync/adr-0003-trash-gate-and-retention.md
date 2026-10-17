# Stage 04g: trash directory mutation coverage and retention risks

Status: incremental guard, not a sync write executor (2026-10-17).

## Changes and regression evidence

- The native Settings **Open Trash** action previously called `ensure_trash_impl` directly, creating `.trash` outside the process-wide Core File Gate. It now calls `files::gate::ensure_trash(core, workspace)`. The guard is dropped before `system::open` and other UI events.
- A Core integration test holds a File Gate lock through one Core instance and invokes trash-folder creation from another Core instance. No directory may appear before release; repeated `ensure_trash` is idempotent.
- `files::trash` now fails closed on a pre-existing `.trash` symlink, NTFS junction or other reparse-root alias before listing, cleanup, purge, restore, move or creating the folder. The alias check is shared by all entry points, not only the UI.
- Unix regression: an external file aged beyond the retention threshold stays untouched with `.trash` symlinked outside workspace. Windows regression: an NTFS directory junction to an external folder is rejected in the same way. Matrix runs the tests on Linux, macOS and Windows.

## Remaining safety limitations

These checks reject **existing** symlink/junction roots; `symlink_metadata` followed by `WalkDir`/`remove_file` is still a pathname-based time-of-check/time-of-use sequence. An untrusted external process may race directory replacement. These functions must NOT be treated as an OS handle-pinned deletion executor or as remote/cloud sync write authorization.

The asynchronous native Settings retention worker uses `gate::cleanup_retention` for timed `.trash` deletion: it scans expiry candidates outside the process-wide mutex, acquires the mutex around **one deletion or empty-directory removal at a time**, rechecks root aliases and file age under lock, then releases it before the next candidate. An outdated scan cannot remove a fresh occupant of a reused pathname. `history::store::adopt_quantum_history` and `history::store::remove_expired` remain direct, separately tracked history-maintenance gaps. No global File Gate lock is held for the whole worker; even item-wise locking cannot guarantee fairness or defeat an external-process TOCTOU path swap. Never place history mutation under the File Gate because that violates `Core::file_mutation_guard` lock-order guidance. A later, bounded cleanup design should isolate short native filesystem operations, check their source path identity, and leave history/DB callbacks outside the lock. History maintenance still needs an independent lock-order and retention-policy audit before any remote write permission.

Existing `.trash` file routing through `files::gate::trash` and `restore` holds the process-wide lock for the file operation; the Directory Gate change here only closes the Settings-created-folder gap.

## CI acceptance

- `Main PR gate` full Rust workspace, core feature and native visual frame shards all green.
- `Sync pinned-read OS checks` Linux/macOS/Windows all green, including `core_file_gate_serializes_trash_creation_between_core_instances` and `unsafe_trash_root` tests.
- Verify no direct activation of S3 apply, deletion or baseline confirmation.

Roadmap: https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11