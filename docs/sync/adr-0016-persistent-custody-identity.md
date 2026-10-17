# ADR-SYNC-016 — Persistent local custody identity for an existing external OS lock

Status: SYNC-04g-3b-7c-b9 — **durable forensic configuration; NO write authority** (2026-10-18).

## Scope

PRs #145–#176 proved kernel-handle identity and negative scope/ACL checks, and demonstrated that a pathname may be replaced with another independently lockable OS object. The ephemeral observation had no cross-restart persistent reference. The existing registered S3 profile stores an explicit shared VaultId distinct from the local workspace UUID.

New private sync/filegate_custody_identity.rs uses an independent app-data SQLite DB with FULL synchronous, WAL and immutable-on-conflict v1 custody_anchor_binding table. Each record binds the registered local workspace UUID, locally observed physical root volume/file ID and path, expected VaultId, external custody scope path and physical ID, and existing lock file physical ID. BLAKE3 checksum and bounded strict JSON catch damage **but are not tamper-proof or authentication**.

SQLite database must be outside/above none of the Core-registered vaults, with an existing canonical parent and without aliased ancestors. A symlinked, reparse or hardlinked existing database refuses. Unknown versions and unrelated databases sharing user_version are refused. Missing record never silently authorizes or repairs.

An initial binding writes the exact first observation transactionally once, refuses all identity conflicts without REPLACE/UPDATE, and rechecks observation after commit. A crash/change after commit preserves the old record and produces an error, not automatic rebinding. Subsequent verification reopens the existing external scope, briefly holds the advisory OS lock, compares all physical IDs, the registered workspace and expected VaultId, then releases the lock before returning. It never modifies vault files or the lock.

When feature sync-s3 is enabled, the optional bind_registered_s3 / verify_registered_s3 entrypoints obtain VaultId only through SqliteRegisteredS3Bindings::load_for_registered for the exact local Core. That pre-existing configuration has an earlier user-reviewed verified-head provenance but is not a cryptographic publisher signature, and these methods perform no network call.

## Acceptance

- Three-OS native host_admission_custody_receipt tests: two independent Core databases with different local UUIDs but the same physical root, external scope, lock ID and declared VaultId; restart both Cores and check durable records.
- Altering the lock path by retaining the original and substituting another file must refuse without changing the old DB receipt; restoration of the exact physical original should verify.
- SQL record checksum tampering, changed VaultId and physical root replacement must refuse without repair.
- S3 feature test binds an existing synthetic, self-consistent S3VerifiedHead through the actual profile registry and verifies the same VaultId and external lock after Core restart. This is **not live remote-authentication evidence**.
- Full Rust/Sync Engine, Docs, S3/MinIO, platform protected originals and Windows/macOS/Linux OS tests required before merge.

## Remaining P0

1. Same-user or administrator attackers may change both SQLite appdata and the external lock or replace its directory entry immediately after verification. Owner/mode/DACL checks are negative forensic controls, not immutable namespace custody.
2. Physical device/inode IDs can be recycled and change across machines, restore or remount. Shared S3 VaultId is not a trusted local OS lock location or cross-device authorization.
3. No live File Gate/HistoryService write uses this record or retains its OS lock. No cross-process writer integration, global lock ordering, bilateral durable PREPARED/PUBLISHED/VERIFIED or crash recovery is proven.
4. Managed cross-vault moves, automatic local sync overwrite/delete, HOLD clearance and confirmed baseline commits remain prohibited.

Next: establish common local custody under a stated cooperative-process threat model, then RAII lifetime-guard all actual File Gate and HistoryService writes with multi-process kill/crash tests. See ADR-SYNC-013/014/015 and https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11.