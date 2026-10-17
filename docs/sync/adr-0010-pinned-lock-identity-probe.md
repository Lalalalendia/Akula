# ADR-SYNC-010 — Pinned identity for an existing File Gate lock object

Status: `SYNC-04g-3b-7c-b3` — controlled native OS identity probe only (2026-10-17). No production write permission.

## Problem

[PR #145](https://github.com/Lalalalendia/Akula/pull/145) proved two-process exclusive lock contention and crash release on Linux/macOS/Windows. But its ordinary path-based `OpenOptions::create(true)` permits a dangerous split-lock: after a lock file is renamed/replaced, two processes can each hold an exclusive lock on different file IDs at the same pathname. A successful OS lock therefore does **not** by itself identify one shared lock object.

## New safety evidence

A `#[cfg(test)]` helper in `sync/filegate_lock_probe.rs` pins the selected root as a directory descriptor/HANDLE, opens only the **already-existing** named lock file relative to that handle, and validates file identity. The experiment reuses native mechanisms from the 04g pinned-read work:

- Unix: `open(root, O_DIRECTORY | O_NOFOLLOW)`, then `openat(pinned_root, lock_name, O_RDWR | O_NOFOLLOW)`; `dev/ino` from the opened descriptor, regular file, exactly one hardlink.
- Windows: existing `pinned_windows::open_root` + a strictly test-only root-relative `NtCreateFile(FILE_OPEN)` helper; reject reparse points, require file/volume ID and exactly one hardlink via `GetFileInformationByHandle`.
- The root is rechecked against its selected pathname before and after the lock, and the lock pathname is opened **again relative to the same pinned root** before and after acquiring the OS lock; its stable object ID must match.
- The lock file is pre-created exclusively by each **test fixture**, not by the probe opener. A missing lock refuses admission instead of silently creating a different mutex object. Tests never unlink a held lock.
- The already-proven two-process checks now run through this pinned existing-file opener: opposite order, conflicting pair, live-child partial-release, crash release, duplicate-root refusal.
- Additional tests cover missing lock, extra hardlink, replacement of the selected root after it has been pinned, and Unix symlink root/lock aliases. No test changes user files.

CI: `oslock_` test filter in the existing Linux/macOS/Windows pinned-read matrix, plus full Rust workspace, Native fidelity and Docs. The exact PR head must be green.

## What this does NOT prove

This is not yet a trusted lock for **registered** Akula workspaces:

1. It is compiled only under `#[cfg(test)]`, with no Core/File Gate/HistoryService production integration. The root is test-selected, **not authenticated against the SQLite workspace registry**. Existing root pinning in read-only sync host admission is separate.
2. The initial persistent lock object's creation, directory entry durability, ownership/access-control policy and crash-safe bootstrap are not designed. No production code may create or delete it.
3. Re-opening the same named file before/after lock acquisition reduces one race window but cannot prevent an adversary unlinking/replacing the name immediately afterward. A surviving handle does not make a pathname globally immutable. Roots on filesystems with unreliable IDs, network shares, removable media and hostile writers require explicit policy.
4. The test does not prove protection against malicious noncooperating OS processes, all ancestor reparse aliases, denial-of-service by a foreign process, or Windows power-loss durability. Even correct cooperative OS locking is not a bilateral database transaction.
5. Do not enable sync overwrite/delete, managed cross-vault file relocation, baseline confirmation, automatic HOLD clearing, or recovery replay from this result.

## Required next gate: actual production custody contract

Before integrating an OS lock into File Gate, define and test a secure persistent per-vault lock anchor under a registered, pinned directory; reject aliases and recreation, preserve lifetime identity, and choose policy for unsafe/network filesystems. All cooperating writers (regular File Gate, HistoryService and native host) must use one canonical lock order and lock the same OS object. A durable bilateral PREPARED/COMMITTED mechanism must be established separately, with crash/kill tests and human-directed recovery.

References: [ADR-SYNC-007](adr-0007-bilateral-cross-vault-recovery.md), [ADR-SYNC-008](adr-0008-filegate-history-hold-admission.md), [ADR-SYNC-009](adr-0009-two-process-lock-probe.md), [Tela canonical PR 00–11 roadmap](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11).
