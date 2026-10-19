# SYNC-APPLY-P0a — S3-bound protected original capture

Status: **read-only local vault; opt-in crate-private preparation only**. Does not modify user files or promote any confirmed baseline. The new wrapper does not authenticate the remote publisher or permit native replace.

## Existing pieces reused

- RegisteredReadAdmission pins the registered root, reserves a closed editor document and checks exact original bytes plus OS file identity under the real Core File Gate mutex.
- SqliteProtectionStore commits an operation/path/hash/root/file-bound original BLOB in WAL + synchronous FULL outside the vault. The old original remains immutable and can be independently reloaded.
- SqliteRegisteredS3Bindings binds the registered local root to an approved shared VaultId/profile; SqliteCustodyIdentityStore holds the pre-existing external interprocess lock throughout the host capture.
- SqliteSyncJournal holds the Staged StageDownload operation and an independently committed previous per-document baseline.

## New host capture predicate

Check the exact remote key/profile/vault, StageDownload operation ID and staged bytes digest, verified prior baseline UUID/hash/profile, and the private journal/protection DB location **before** saving original bytes. Repeat the journal/profile checks under the cooperative OS lock before and after a pinned original read/SQLite FULL commit.

No storage of untrusted input in the user vault. An uncertain post-lock check may have left an intact protective copy but NEVER allows replacement, replay, rollback, deletion or baseline advancement. Journal stays Staged; generic mark_applied/verified/commit StageDownload remains forbidden.

## Tests

Real Core registration and external OS custody fixture: capture and reopen exact original bytes, idempotence of one operation, unchanged destination, unchanged Staged phase; reject missing operation, wrong S3 profile, live editor and externally changed original; reject replaced external lock with original unchanged.

## Subsequent gates

1. Implement separately verified S3 update staging with full version DAG and per-document descendant baseline, no skipped tombstone or competing heads. This P0a fixture only tests the local protection predicate; simulated staged bytes alone are never write authority.
2. Design OS-handle-relative atomic replace, durable original publication receipt and HOLD on ambiguous crash/lost ACK (Windows/macOS/Linux). Preserve exact preimage and bound physical root/custody identity through the transaction.
3. Only after independently proven OS durability and pinned readback may the host-only journal checkpoint be advanced exactly once by CAS. A remote S3 observation anchor is not a per-document applied baseline.
4. Reject missing/untrusted installation broker for adversarial same-user threat claims; existing cooperative OS lock is insufficient by itself. No automatic sync/delete/GC.
