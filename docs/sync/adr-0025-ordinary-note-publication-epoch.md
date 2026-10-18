# ADR-SYNC-025 — Ordinary note publication lease across primary and HistoryService

Status: scoped P0 closeout after ADR-SYNC-024; still not complete File Gate coverage or sync release authorization.

## Race witnessed

An ordinary File Gate `write` or text-note `create` previously released `PROCESS_FILE_MUTATIONS` immediately after changing the primary note, then acquired `PROCESS_HISTORY_FILES` to save a version. Activation could acquire those mutexes in the gap, turn durable participant HOLD on and expose a guarded route. The old ordinary writer would then have changed primary bytes but received a direct HistoryService HOLD error later, while its File Gate API still returned a success for the file operation.

## Bounded remedy

- Under `sync-s3` only, introduce a process-global `RwLock<()>` publication epoch. An ordinary File Gate text write/create holds a **read lease spanning both primary-file replacement and the complete HistoryService call**; guarded File Gate writes use their independently verified external lock and deliberately do not need this ordinary lease.
- The S3 cooperative activation path first checks/arms DocumentHub sessions **before acquiring any process-level activation mutex**, then acquires an exclusive epoch lease, followed by the process File Gate mutex and finally the HistoryFiles mutex. It commits the participant record with SQLite FULL sync while those activation barriers are held, then publishes the guarded route.
- Event notifications, search callbacks and file-change announcements must occur **after the ordinary publication lease is dropped**; `record` delegates to a shared deferred `report_record` helper to preserve pre-existing success/error events without invoking hosts under the new lock.
- The Rust regression forces a real ordinary File Gate write to publish primary bytes while `HistoryService` is artificially held. Its exclusive activation epoch must be blocked until that writer also finishes the history attempt. This tests the exact primary/history gap, not just geometric mutex acquisition.

## Remaining gates

Only `File Gate::write` and `File Gate::create` participate in the new two-stage publication lease in this slice. Rename, move, trash, restore and other mutators may still release File Gate before history and require the same end-to-end lease or a separately proved alternative. Avoid adding recursive/nested publication leases without checking callback/reentrancy; process RwLock fairness and p50/p95 write latency must be measured. Direct HistoryService was covered by ADR-SYNC-024, with the pinned-custody exception sealed inside the guarded writer.

Still a **single-process cooperative** mechanism: different appdata roots, independent OS processes, malicious same-UID actors, direct filesystem writes and loss/replacement of the external custody namespace remain outside proof. Public S3 auto-sync, overwrite/delete, baseline promotion, garbage collection, and HOLD clearance remain disabled.
