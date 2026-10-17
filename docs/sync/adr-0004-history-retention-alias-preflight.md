# 04g-3b: history retention / legacy migration alias preflight

Status: proposed safety hardening; no cloud sync write authority (2026-10-17).

## Root cause

`history::store::remove_expired` recursively enumerated `.aquilum/history` with `WalkDir`, discarded traversal errors using `filter_map(Result::ok)`, then called ordinary file deletion. `adopt_quantum_history` similarly traversed `.quantum/history` and moved versions to `.aquilum/history` after `create_dir_all`. A pre-existing symlink/Windows NTFS junction at `.aquilum`, `.quantum`, `history`, or a nested source/destination directory can redirect these pathname-based operations outside the intended workspace.

## Safeguards introduced

- Check every **existing directory ancestor** between the selected base (vault or `.trash`) and the history root with `symlink_metadata`. Deny symbolic links, non-directory roots and Windows `FILE_ATTRIBUTE_REPARSE_POINT` directories. An absent legacy history folder remains a harmless no-op.
- Preflight **all** reachable entries with `WalkDir::follow_links(false)` before mutating. Symlinks, reparse points, unusual node types or traversal errors now abort the entire housekeeping pass rather than being ignored or partially applied.
- Migration verifies the source and destination trees, nested destination parent components and the source file type. Existing destination versions are never overwritten. Inspect destinations using `symlink_metadata` rather than `exists()` so dangling aliases cannot be mistaken for a free filename. Recheck the relevant prefixes before each rename.
- Retention similarly requires an error-free tree preflight and a second pre-deletion scan. Its public legacy API returns `0` on unsafe trees and records an error to stderr; `adopt_quantum_history` returns without moving bytes and logs the refusal.
- Real-OS regression tests cover Unix `.aquilum/history` and `.quantum` symlinks and nested destination aliases; Windows junction roots and nested destination reparse points. The tests verify outside files and any unexpired/original versions survive.

## Boundary: not a verified mutation authority

This is a **static alias preflight** and does not make pathname-based `rename`, `remove_file` or `WalkDir` immune to external-process rename/junction swaps after the check. It is also not a durable/atomic all-or-nothing multi-file migration; an independent external mutation or disk error can still interrupt it. The process-wide Core File Gate only coordinates cooperating Akula operations, not other processes. Do not reuse these helpers to implement remote sync apply, delete, overwrite or baseline confirmation.

Other history mutation APIs (`write_version`, name writes, version edits and restores) still need dedicated host ownership, path confinement and write-side locking review. The Settings retention worker itself still invokes housekeeping without holding the File Gate; avoid taking the process-wide mutex around entire unbounded history migration and SQLite cleanup. Future work: bounded operations, correct lock ordering and OS-pinned root-relative filesystem calls.

## CI acceptance

- The existing required `Main PR gate` passes all workspace Rust tests, sync-engine features and native pixel/frame fidelity.
- The existing pinned-read workflow runs all history-store tests on Linux, macOS and Windows, including alias regression fixtures, without introducing a new workflow.
- No remote I/O capability, sync scheduler or local write authorization is enabled.

Roadmap: https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11