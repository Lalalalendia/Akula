# ADR-SYNC-026 — File Gate relocation, trash and restore across a full history epoch

Status: stacked on ADR-SYNC-025 / PR #245, internal opt-in safety only. No public S3 sync authorization.

## Gap

An ordinary File Gate rename/move/trash/restore can commit primary filesystem changes and release the process File Gate mutex before transferring the corresponding history. The S3 enrollment fence introduced by ADR-SYNC-024 could enter that interval, publish durable participant HOLD and cause the preceding HistoryService transfer to fail. ADR-SYNC-025 addressed note write/create, not these operations.

## Narrow integration

- In sync-s3 builds, take the same process-local read publication epoch before the primary rename, same-vault move, trash or restore. Retain it through every matching HistoryService move/trash/restore transfer and through link-rewrite history version saves.
- The guarded-participant activation path retains the exclusive epoch acquired in ADR-SYNC-025 before File Gate, HistoryFiles and the durable participant insert. It therefore waits for these older publication transactions to finish.
- Defer `NoteHistoryMoveFailed` and `NoteHistoryChanged` events, relocation announcements, search notifications and document reconciliation until after the read epoch is released. Never invoke a host callback from within the epoch.
- Preserve old File Gate refusal and history partial-failure semantics. A relocation that changed primary bytes but failed history is still a recovery issue, not a silently retryable protected remote edit.
- Deterministic regression holds HistoryFiles while running real rename, move, trash and restore operations. After the primary bytes change and the primary File Gate mutex is free, assert the exclusive enrollment epoch cannot be acquired. Release HistoryFiles, join the writers, validate bytes and history.

## Limitations

This is a cooperative **single-process** publication epoch. Standalone HistoryService, direct filesystem mutations, other appdata / OS processes, hostile same-UID writers, and root/custody namespace swaps can evade these Core-local checks. It is not a distributed transaction or a two-vault recovery journal.

Long folder operations can hold a read lease while processing many history entries. Measure contention, p50/p95 latency and callback reentrancy before enabling interactive S3 enrollment. No S3 overwrite/delete/GC, baseline advancement or automatic HOLD clearance is granted by this change.
