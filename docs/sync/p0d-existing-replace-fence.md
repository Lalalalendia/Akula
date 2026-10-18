# SYNC-APPLY-P0d — durable non-replayable existing-file attempt fence

Status: **read-only user vault**, no native file mutation, no publication receipt, no baseline advancement.

P0b (#303) verifies an unambiguous S3 fast-forward; P0a (#300) preserves the original under Core custody; P0c (#316) stages the new immutable blob in private SQLite. An existing file still must never use the additive-only no-replace path. The new P0d fences one potential attempt separately in the SAME FULL/WAL sync journal.

The v7 table keeps the Staged operation, complete original preimage metadata, original OS root/file identities and confirmed previous baseline, and validates the exact previous committed path/old digest. A conditional SQLite INSERT rechecks current baseline, staged hash and intent digest. The unique key is (VaultId, portable path, previous revision). This blocks a second attempt for the same baseline after process loss, but allows later versions only after a separately proven baseline promotion. Foreign documents claiming the same portable path are rejected.

Only private metadata are stored. Original bytes stay inside the protected appdata SQLite store. The forensic reader checks record digest/fields but **cannot grant write authority**. A returned record or matching hash does not authorize an OS rename, replay, restore, deletion or sync confirmation. Legacy 0–6 schemas migrate through v6 to v7 on writer open; read-only UI never upgrades a DB.

Required acceptance: two SQLite connections, crash-restart retention of the claim, wrong preimage/hash/root/file identity, corruption refusal, v6 migration, unchanged old baseline, and continued StageDownload->Applied rejection. Follow-on P0 needs a host-custody bridge that revalidates both staged bytes and exact protected original under an external OS lock, then platform-native durable replacement with fault injection and a separate trusted receipt.

## Real Core bridge

The same PR now also provides an explicitly opt-in S3-bound preparation call in `s3_preimage_capture.rs`. It checks the approved registered S3 binding, private WAL/cache locations and the cached new blob, then holds the existing external OS lock during pinned old-file identity checks and the one-time SQLite insert. Both missing original archives and a same-content inode replacement must fail before a claim; after any committed claim failure the operation remains HOLD. This is still a read-only preflight, never an OS replace receipt or baseline promotion.

Synthetic migration tests for versions 1–5 remove v7-only tables when constructing their historical database fixtures; production migration deliberately uses strict CREATE TABLE to reject inconsistent existing schemas, rather than silently accepting an already-present table under an old version.
