# ADR-SYNC-009 — Two-process OS lock probe before bilateral File Gate

Status: SYNC-04g-3b-7c-b2 test-only OS proof (17 October 2026). NOT a user-data write authorization.

## Why

PR #124 rejects cross-vault primary moves; #137 exposes persistent source/destination HOLD after restart; #141 stops ordinary File Gate writes while a registered Core has pending or corrupt history. Their safety still relies on per-process mutexes and pathname markers. Separate Akula processes can acquire different views without a real OS interprocess admission protocol.

## What the experiment establishes

- Under `#[cfg(test)]` only, `core/src/files/gate/interprocess_probe.rs` creates two disposable `tempfile` vault roots, canonically orders the two paths and calls `std::fs::File::try_lock()` on a persistent empty file per root.
- An actual child OS process (the compiled Rust unit-test executable) attempts to acquire the same pair in reverse order. A live parent's lock must deny access, and after release the same child probe can acquire both.
- A collision on the second root must release the already acquired first-root lock **while the declining child process is still alive**. The parent successfully locks that first root before killing the child, so normal process-exit cleanup cannot cause a false positive. This proves release of partial lock handles, NOT rollback of data.
- A child process holding both handles is killed by the parent. Both handles must become available to the parent afterward without removing the lock files. Lock release after process death does not confirm an interrupted operation was committed.
- A pair where both names resolve to the same root is rejected. This does not establish trusted root identity against hostile filesystem aliases.
- A dedicated `oslock_` step is required in the existing Linux/macOS/Windows pinned-read matrix. No new workflow, dependency or production feature flag is introduced.

## Explicit limitations

The probe opens lock files by ordinary paths in disposable, controlled test directories. It has NO trusted root-relative open, no symlink/junction/reparse or hardlink swap defense, no validation of Unix `(st_dev, st_ino)` or Windows volume/file ID after open, no interaction with a real Akula Core or HistoryService, and no evidence of power-loss durability. OS file locks serialize only processes cooperating on the same stable lock object. Never treat either an unlocked file or a positive `try_lock` as authority for sync overwrite/delete or cross-vault moves.

## Next production gates

1. Develop a trusted per-vault lock object created/opened relative to a pinned registered root; forbid substitution and detect hardlinks or changed file identity on Windows/macOS/Linux.
2. Define and enforce lock order across File Gate, HistoryService, native editor buffers, Core instances and independent processes. Ensure never to acquire history mutex under File Gate or emit UI callbacks while holding a cross-process lock.
3. Revalidate registrations, source/destination identities, history inventories and both HOLD journals under both locked roots; persist bilateral PREPARED before primary mutation.
4. Test crash/kill at each transition, collision, stale hash, independent source-only/destination-only Core, and operator-directed recovery. Releasing a lock after crash is not successful replay.
5. Benchmark SQLite/FS/HOLD check overhead on the normal note-save path before any production integration.

References: [ADR-SYNC-007](adr-0007-bilateral-cross-vault-recovery.md), [ADR-SYNC-008](adr-0008-filegate-history-hold-admission.md), [canonical Tela roadmap](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11).