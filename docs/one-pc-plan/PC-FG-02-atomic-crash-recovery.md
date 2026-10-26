# PC-FG-02 — atomic write crash recovery

**Task:** FG-03/07, atomic save, temporary files, WAL/SQLite and termination at filesystem phases.
**Notion:** [PC-FG-02](https://app.notion.com/p/40332a84beec81b29914cd38f9e7389b)
**Disposition:** locally reproduced on one Windows PC. The test-only atomic-write probes and this report are published in a Draft PR stacked on PC-FG-01 #703; exact PR head and CI state are recorded on the linked Notion task.

## Change

Added `#[cfg(test)]` pause points inside the existing shared `write_atomic_bytes_impl` publisher. The child test calls the real atomic publisher against a named disposable note. The test parent uses `Child::kill` to terminate the writer, then starts a fresh verifier process. No production writer or environment-controlled behavior is added.

The five Windows process-termination points produced these cold-read results:

| Termination point | Primary file after restart | Sibling temp left by abrupt termination |
| --- | --- | --- |
| Temp file opened | Previous 26-byte version | Empty temp |
| Half of 1 MiB written | Previous version | 512 KiB temp |
| Full temp write, before CAS | Previous version | Full 1 MiB temp |
| CAS passed, before commit | Previous version | Full 1 MiB temp |
| Commit returned | Complete new 1 MiB version | None |

Previous version BLAKE3: `6a142e66da92ce6e23b171a4fcffb227361525fd4ca75a6596fcf91ba968addd`.
New version BLAKE3: `a9a8f13c55cd434af8a6c26c4facb45c342bfc361ebded32866f86df18e3db82`.

The stale-CAS negative test passed: the external bytes remained unchanged and the ordinarily dropped temp file was removed. Existing local sync crash checks also passed: the V7 / primary / HistoryService / V8 termination matrix reopened in a new process without replay, and the partial-persistence cases (missing V8 row, missing history version, changed primary) remained fail-closed with the prior baseline and protected original intact.

## Checks

Freshly re-run against the current worktree on 2026-10-24. Commands used Rust `1.97.1`, `--locked --offline`, `CARGO_INCREMENTAL=0`, one build job, Cargo home `.artifacts/one-pc-cargo-home`, and target directory `D:\mobakula\.artifacts\cargo-native`. The atomic kill test ran with default features; the SQLite/WAL tests used `--features sync-s3`. Windows fixtures used the ACL-restricted temp root `.artifacts/one-pc-checks/PC-FG-01-20261024-current/restricted-temp` (current user, SYSTEM, Administrators).

- `files::document::atomic_kill_tests::` — **4 passed** (includes five hard-kill phases, cold verifier processes, stale CAS and both child entrypoints).
- `sync::s3_reserved_replace::tests::p0g3d08c_real_writer_abort_matrix_has_no_automatic_replay` — **1 passed**.
- `sync::s3_reserved_replace::tests::p0g3d08d_partial_persistence_domains_fail_closed_after_v8` — **1 passed**.
- `git diff --check` — **passed**.

The build emitted existing warnings from the broader Core test crate (27 without `sync-s3`, 39 with it); all selected tests completed successfully.

Current logs:

- `.artifacts/one-pc-checks/PC-FG-02-20261024-current/atomic-phases-current.log`
- `.artifacts/one-pc-checks/PC-FG-02-20261024-current/sqlite-wal-abort-matrix-current.log`
- `.artifacts/one-pc-checks/PC-FG-02-20261024-current/sqlite-partial-domains-current.log`

The earlier logs in `PC-FG-02-20261024/` predate the latest source edits and were not used for this acceptance. The fresh runs above use the same current worktree and exact Git `HEAD` recorded below.

## Limits and repository state

Abrupt pre-commit termination leaves the dependency's named sibling temp file behind; the tests confirm it never replaces or alters the visible primary document. Automatic scavenging is not added because safe removal must not race another writer. The test kills a process on the current Windows installation; it is not a Windows restart, QEMU hard-off, physical power cut, or proof of storage-controller/NTFS power-loss durability. It does not stop execution inside the dependency's `sync_all` or rename system calls.

The tests were not submitted to GitHub Actions. The workflows `filegate-history-stage-profile.yml` and `sync-native-vm-hardoff.yml` declare `workflow_dispatch`, but no run was dispatched; no merge/Main PR gate is claimed for these uncommitted changes. Read-only GitHub searches found no PR titled or keyed for PC-FG-02 and no identical ordinary atomic-write recovery implementation. Open Draft #632 adds a separate S3 prewrite EIO/ENOSPC test leaf; it does not implement the filesystem process-kill phases. Open Draft #694 is the installed P2P integration and does not overlap this task's source files.

- Branch: `main`.
- Exact current `HEAD`: `530fe3d6794ee28d2c7098af1266ff5340903543` (unchanged; PC-FG-02 source and this report remain uncommitted).
- The other existing local changes, including PC-FG-01, remain in the working tree.
- Notion status was left as-is.

**Next plan task:** PC-FG-03 — history revision identity, BLAKE3, and exact restore behavior.


## Повторная проверка и публикация — 2026-10-24 11:53:26 UTC

- Atomic filesystem phases: fresh run on this machine — **4 passed, 0 failed**. The test killed the writer at all five seams and cold-read verified old-or-complete-new content. Log: `.artifacts/one-pc-checks/PC-FG-02-20261024-publish/atomic-kill-phases.log`.
- SQLite/WAL real-writer abort matrix: **1 passed**, and partial persistence domains: **1 passed**, using the prior successful same-worktree logs `.artifacts/one-pc-checks/PC-FG-02-20261024-current/sqlite-wal-abort-matrix-current.log` and `sqlite-partial-domains-current.log`. The source files for those existing tests were not changed after that run. A fresh repeat was stopped during the long feature rebuild before tests began; it is not reported as a fresh pass.
- `git diff --check` on the relevant local source files: **passed**.
- The new code is limited to `#[cfg(test)]` atomic write phase probes and their child-process tests in `core/src/files/document.rs`. The production atomic publisher is unchanged. The PR is Draft, stacked on #703; no merge, manual Actions dispatch, or physical power-loss claim.
