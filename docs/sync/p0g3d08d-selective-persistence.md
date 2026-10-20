# SYNC-P0g-3d-08d-1 — independently lost persistence domains (test-only)

**Status:** isolated laboratory matrix, **not** hard-off acceptance and **not** a native durable receipt.
**Dependency:** [#444](https://github.com/Lalalalendia/Akula/pull/444), merged as `596bfdb`.
**Source:** `aquilum-app/core/src/sync/s3_reserved_replace_tests.rs`, test `p0g3d08d_partial_persistence_domains_fail_closed_after_v8`.
**Platforms:** Windows/NTFS and Linux CI (macOS remains Frozen/P2).
**Activation:** `cargo test --locked -p aquilum-core --features sync-s3 p0g3d08d_partial_persistence_domains_fail_closed_after_v8 -- --nocapture --test-threads=1`.

## Why this test is needed

The protected original (`originals.sqlite`), one-shot V7 / V8 (`journal.sqlite`), primary file (`vault/Reserved.md`) and HistoryService version (`vault/.aquilum/history/...`) are **independent persistence domains**. SQLite `WAL+synchronous=FULL` does not make all of them one atomic transaction.

On Akula's pinned `atomic-write-file 0.3.0`, Unix `renameat` includes a parent directory `fsync`, while the Windows generic implementation uses `std::fs::rename` without establishing an equivalent namespace durability contract. This *risk* is source-confirmed; it is not an observed loss under a physical power cut.

## Exact experiment

For each case, build a **new disposable** registered Core fixture; execute one real guarded File Gate + immutable external HistoryService + V7/V8 write. Close the original Core and all writable SQLite connections. Independently reopen the complete fixture in another OS process and require `LocallyConsistentHold` (never `ConfirmedBaseline`).

Then remove **exactly one** correlated durable observation and invoke a **new** independent OS process:

| Image after hypothetical mixed-domain loss | Expected recovery | Additional invariants |
| --- | --- | --- |
| V8 row absent; V7, original, primary and history intact | Deny | No second V7 attempt |
| Named immutable HistoryService version absent; V8 and primary intact | Deny | No synthetic history witness |
| Primary directory name points at original bytes again; V8/history retain candidate | Deny | No false match from unchanged metadata |

All three require `StageDownload` remains `Staged`, the previous `ConfirmedBaseline` stays unchanged, original BLOB bytes stay readable and `mark_applied` returns `LocalDownloadNeedsNativeReceipt`. The control and negative observations run in independent OS processes, not a fresh `Core` in the writer process.

The test intentionally mutates **only a disposable fixture** after all writer handles close. It never touches user files or production synchronization.

## Evidence that this does NOT provide

- No block-device power loss, host writeback/cache test, QEMU hard-off, NTFS flush witness, or power cut.
- No inter-volume flush ordering proof; no assurance that V7 and original survive when on different physical devices.
- No new `PreservationReceipt` or automatic S3 apply, ACK, deletion, original cleanup, `mark_applied`, or `ConfirmedBaseline` promotion.
- The positive `LocallyConsistentHold` control means *currently readable correlated data*, not that it would survive a force-off.

## Next P0g-3d-08d-2 gates

1. **Pinned writer audit:** inspect Windows `AtomicWriteFile::commit`, the underlying `std::fs::rename` behavior and exact NTFS durability guarantees; inspect Unix temporary inode/file fsync + rename + parent directory fsync, and `HistoryService` initial directory creation. Record independently mounted app-data and vault volumes.
2. **Guest-side instrumentation:** capture stage witness **out of band** (hypervisor serial/host control channel). It cannot be trusted if written only to the tested disk. Distinguish a VM force-off from an actual electrical storage power cut.
3. **Linux forced VM power-off:** isolated disposable guest/disk images, barrier-by-barrier kill, reboot and **read-only** independently reopened evidence, comparing expected and actual state. Include failures at partial directory creation and WAL recovery.
4. **Windows NTFS forced VM power-off:** same test with independently mapped volumes and known disk cache mode. Fail the gate if namespace durability cannot be established; do not assume `FlushFileBuffers` of the temporary file also flushes a replacement directory entry.
5. **Release gate:** even a green VM matrix remains a separate evidence record from a real physical power-cut/storage test. Do not enable S3 writes, ACK or `ConfirmedBaseline` without a separately reviewed authoritative native receipt protocol.

**Pass criterion for this PR:** exact-head Windows/Linux Reserved S3, Rust and Main PR gate, all three negative OS-process probes + positive control. No production behavior or schema modifications.
