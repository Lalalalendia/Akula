# ADR-SYNC-020 — Crash-durable native document write intent

Status: **internal opt-in protocol**, NOT user-facing editor activation or S3 sync authority. Date: 2026-10-18.

## Failure being closed

Merged #219 creates an in-memory HOLD after `DiskError { code: "needs_review" }`. The process can instead crash after a real File Gate publication and before it receives/reports any error. A post-error SQLite insert therefore does not survive this failure.

## Crash-consistent protocol (DocumentHub / app-data)

- `DocumentStore` schema v2 adds `write_intents` keyed by the existing canonical document identity; v1 migrates in place without rewriting CRDT replicas, updates or sync points.
- For **an explicitly armed registered root only**, `SessionIo::begin_guarded_write` commits an **insert-only** record with old BLAKE3 CAS hash and proposed new hash using SQLite WAL `synchronous=FULL` **before** any primary Markdown writer runs. It refuses an existing unresolved intent.
- The existing File Gate performs primary CAS publication and HistoryService independently; the newly inserted row is NOT a success receipt and never grants overwrite, delete or history authority.
- After a successful File Gate/HistoryService ACK, the DocumentSession must persist its sync point. **Only after both succeed** does `SessionIo::complete_guarded_write` retire that exact row, again using `synchronous=FULL`. A crash during any intermediate stage leaves the hold, including crashes after file/SQLite sync-point but before row deletion.
- During `DocumentSession::open`, `HubIo::unresolved_native_write` queries this app-data record **before reading or merging disk**. An unresolved row rejects re-opening with `needs_review`, preserves the app-data CRDT log and prevents silent replay. Even identical file hashes cannot clear a pending row. A missing/unreadable app-data store is fail-closed for DocumentHub openings.
- Guarded writer errors, missing sync-point confirmation or intent retirement failures become sticky in-memory `needs_review`. Watcher reconciliation and write timers already stop via #219. `DocumentStore::forget/relocate` refuse silently discarding a held key; these DB-level checks do not guard standalone File Gate rename/trash.
- The opt-in `DocumentHub::arm_durable_intents_for_root` checks a pre-existing healthy v2 document store and a physical root, and refuses arming with any open DocumentHub document or potentially unsent CRDT buffer inside that root. It holds the sessions mutex through root registration. The future caller must additionally quiesce separate native/standalone writers. **No default root is armed.** Ordinary offline saves do not issue FULL-sync intent commits.

## Tests / known limitations

- Schema migration v1 → v2 preserves existing CRDT bytes.
- Persistent intent survives SQLite close/reopen, refuses duplicate arm, mismatched success ACK, silent forget or relocation; a matching confirmed commit can explicitly retire it.
- Two Core opens using one app-data SQLite database: simulate primary file change with no ACK; the second Core cannot open/reconcile the ambiguous note, and its previous confirmed sync point is not promoted.
- Fake `SessionIo` proves that successful primary+sync-point confirmation retires the intent; a sync-point failure leaves it and blocks reopening.

**Not released for ordinary editor saves:** Draft #215 must call `core.documents.arm_durable_intents_for_root` **before** registering its File Gate guarded editor route, and must use the same identity key for each armed path. If S3 profile/custody verification or the durable journal is unavailable, refuse activation without lock-free fallback. This PR adds the protocol and cross-platform tests but does not automatically activate it.

No operator-reviewed clearance UI is available yet; HOLD rows intentionally have no automatic expiry. Future work must tie this app-data record to verified workspace/VaultId custody and prohibit direct rename/trash or other file mutations until a reviewed resolution. This mechanism does not establish Windows power-loss durability for the *native primary file*, adversarial same-user namespace resistance, remote `StageDownload` baseline confirmation, E2EE, or conflict-resolution permission.
