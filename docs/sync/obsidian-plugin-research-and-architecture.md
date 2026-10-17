# Akula cloud sync: Obsidian-plugin source review and implementation contract

**Status:** architecture research only. No sync implementation, account integration or credentials added.
**Baseline:** Freaction/Aquilum 0.3.9, Akula main at 10d22965805235d08b94fe7cbc626b1fa2867c09.
**Goal:** local-first, two-way vault synchronization over Yandex Disk, Google Drive and S3-compatible object storage.

## 1. Source-code references (verified 2026-10-16)

| Reference | Source | Mechanism | License |
| --- | --- | --- | --- |
| BYOC | https://github.com/winters27/obsidian-byoc | Generic local/remote FS adapter; three-provider integration; three-way baseline/planner/syncer | Root MIT; src/ Apache-2.0 |
| Cloud Drive Sync | https://github.com/sm-merlot/cloud-drive-sync | Separate CloudProvider / LocalFileSystem / SyncEngine / SyncUIPort / SyncStateStore; Google & S3 | MIT |
| Yandex Disk Sync | https://github.com/Ivansushkin/obsidian-yandex-disk-sync | Causal SHA-256 state, canonical remote index, tombstones, guarded downloads, durable mutation coordination | MIT |
| Remotely Save | https://github.com/remotely-save/remotely-save | Mature S3 adapter and sync-algorithm documentation | src/ Apache-2.0; pro/ PolyForm Strict. Do NOT copy pro/ code |

Precise code paths:

- BYOC planner: https://github.com/winters27/obsidian-byoc/blob/master/src/sync/planner.ts
- BYOC executor: https://github.com/winters27/obsidian-byoc/blob/master/src/sync/syncer.ts
- BYOC S3: https://github.com/winters27/obsidian-byoc/blob/master/src/fsS3.ts
- BYOC Yandex: https://github.com/winters27/obsidian-byoc/blob/master/src/fsYandexDisk.ts
- BYOC Google: https://github.com/winters27/obsidian-byoc/blob/master/src/fsGoogleDrive.ts
- Cloud common provider interface: https://github.com/sm-merlot/cloud-drive-sync/blob/main/packages/core/src/providers/cloud-provider.ts
- Cloud sync engine: https://github.com/sm-merlot/cloud-drive-sync/blob/main/packages/core/src/sync/sync-engine.ts
- Cloud Google API: https://github.com/sm-merlot/cloud-drive-sync/blob/main/packages/obsidian-plugin/src/providers/google-drive/google-drive-api.ts
- Cloud Google OAuth: https://github.com/sm-merlot/cloud-drive-sync/blob/main/packages/obsidian-plugin/src/providers/google-drive/google-drive-auth.ts
- Cloud S3: https://github.com/sm-merlot/cloud-drive-sync/blob/main/packages/core/src/providers/s3/s3-provider.ts
- Yandex sync conflicts: https://github.com/Ivansushkin/obsidian-yandex-disk-sync/blob/master/docs/CONFLICTS.md
- Yandex transaction and recovery rules: https://github.com/Ivansushkin/obsidian-yandex-disk-sync/blob/master/src/sync/canonical/transaction.ts
- Yandex HTTP adapter: https://github.com/Ivansushkin/obsidian-yandex-disk-sync/blob/master/src/api/yandex-client.ts

## 2. What the implementations reveal

**BYOC:** walk local + remote, load a previously confirmed sync baseline, classify each path into push, pull, delete, rename, equal or conflict; sort folder creations before files and folder deletions after; execute, preserve conflicts and only commit confirmed operations. Its planner relies partly on mtime/size with tolerances and cannot be treated as a universally correct content identity oracle. Important safeguards: dry-run; cap on destructive changes; do not commit an initial baseline after partial failure; retain old baseline entries for failed operations; explicitly protect undecryptable entries against false remote deletion. S3 folder objects may be synthetic; multipart ETag is not necessarily a file hash.

**Cloud Drive Sync:** good Rust-portable architectural separation via CloudProvider, LocalFileSystem, SyncEngine, SyncStateStore and SyncUIPort. Google Drive files have stable IDs rather than paths; list pagination and a dedicated app-owned folder marker are needed. Google uses drive.file OAuth scope in the examined implementation. S3 supports SigV4, key prefixes, pagination and multipart uploads; its multipart ETag must not be treated as an MD5 checksum. It also has user-visible initial-sync modes and per-file conflict resolution.

**Yandex Disk Sync:** best reference for data safety. Plaintext SHA-256 local fingerprints, remote physical fingerprints and causal baseline, explicit deletion tombstones, pending local operations, a single coordinator, durable recovery and conflict copies. It verifies remote metadata before and after download, and refuses to apply an outdated action if local content changed while it was in flight. Canonical index commits have explicit stages and ambiguous-outcome recovery. Note: a remote index lock/move alone is not a universal atomic CAS across all providers; do not assume concurrency safety without a provider-specific proof.

## 3. Recommended Akula architecture

New Rust module under aquilum-app/core/src/sync/, independent of the native UI:

- model.rs: VaultIdentity, DeviceIdentity, FileVersion, RemoteVersion, SyncState, SyncAction, Conflict, Tombstone; normalized vault-relative paths.
- local.rs: LocalFileSystem adapter with atomic writes, protective copies, symlink/traversal denial, Windows case/Unicode collision checks and watcher integration.
- provider.rs: RemoteStore abstraction for paginated list, fetch, put, delete/trash, folder discovery, object version metadata, optional conditional writes, and resumable large uploads.
- providers/yandex.rs: Yandex REST API / OAuth, upload/download signed URLs, server fingerprints and rate-limit retries.
- providers/google.rs: Drive v3 / installed-app OAuth2 PKCE, drive.file or carefully disclosed broader scope, app-owned root folder, stable file IDs, pagination and token refresh.
- providers/s3.rs: AWS-compatible SigV4 via maintained Rust SDK, custom endpoint/region/bucket/prefix, multipart upload, conditional writes where supported, S3 Versioning integration when available.
- journal.rs: durable local SQLite queue: pending operations, confirmed per-file baseline, remote ID/version mapping, conflict copy inventory and resume cursor; one state namespace per (vault, provider profile, remote root).
- planner.rs: compare Local L, Remote R and Baseline B using verified content identity. Never infer deletion from network errors, filtered listings or absent provider authorization.
- executor.rs: bounded transfers, retries with exponential backoff+jitter, cancel/resume, revalidate before any destructive local write or remote delete, persist the baseline only after verified success.
- ui.rs / native integration: one active provider per vault for v1; setup, connection test, initial sync preview, status, progress, conflict resolution, pause, one-way recovery and last-error details.

**Important:** Akula already has WorkspaceWatcher, documents/conflict handling, Core file-gate writes, HistoryService and SettingsManager. Integrate those instead of introducing a second filesystem-authority path. Sync should treat search indices, SQLite state, history caches, auth tokens and app internals as device-local. Default sync scope: Markdown + binary attachments; dotfiles / .obsidian settings optional explicit opt-in.

## 4. Conservative decision matrix

| Local vs baseline | Remote vs baseline | Initial policy |
| --- | --- | --- |
| Same | Same | No-op |
| Changed | Same | Upload with precondition; confirm remote fingerprint |
| Same | Changed | Download to temporary file; recheck local; create backup; atomic replacement |
| Changed | Changed | Preserve two versions and surface a conflict; never silently choose by newest mtime |
| Deleted | Same | Queue tombstone/trash with retention; do not hard-delete during first sync |
| Same | Deleted | Safe local trash only after confirmed causal deletion; otherwise stop |
| Deleted | Changed (or inverse) | Conflict; preserve surviving content |
| No baseline + both present | No baseline | Preview first-sync upload/download/merge; no blind overwrite |

No automatic provider-to-provider mirroring in v1: separate baselines and remotes prevent feedback loops. Provider switching begins as an explicit migration / new initial sync.

## 5. Reliability and security acceptance

1. Never ask for or embed a user's secrets in the repository. Store OAuth refresh tokens and S3 credentials using the OS secure credential store; do not write tokens to vault files or logs. Google desktop auth uses PKCE/loopback or equivalent native-app flow, not a hardcoded distributable client secret.
2. Snapshot+hash each file; use BLAKE3 or SHA-256 for local content and verify downloaded bytes. Remote ETag is only a provider version hint (and S3 multipart ETag is not a whole-object MD5).
3. Guard local writes against a concurrent editor save, and preserve both versions on conflict. Avoid sync loops by suppressing only events identified as resulting from a confirmed internal write.
4. First sync defaults to non-destructive preview. Block large-scale deletes/overwrites; never treat 403/429/5xx, incomplete list pagination, missing refresh permissions or decrypt failure as remote deletion.
5. Maintain confirmed baseline in SQLite only after transfer+verification. Persist pending ops before network mutations; test mid-upload abort, crash after remote success but before local commit, retry idempotency.
6. Preserve tombstones for a retention period and use provider trash/versioning when possible. Do not silently overwrite existing files, configs or conflict copies.
7. Support Cyrillic/emoji/Unicode NFC, reserved Windows names, case collisions, very large attachments, path traversal and symlinks. Do not let remote object keys escape the vault.
8. Test at least two devices editing offline and syncing in both orders. Test deletes, renames, upload-then-disconnect, conflict copy collisions, 401 refresh, 403, 409, 412, 429, 5xx, full pagination and mid-transfer cancellation.
9. S3 adapter matrix: AWS S3, MinIO and one compatible service (Yandex Object Storage/Cloudflare R2). Google: empty app-owned folder and existing vault under allowed scope. Yandex: app_folder vs full-disk scope explicitly configured.
10. Per-provider conditional-write semantics differ. Where atomic compare-and-swap cannot be guaranteed, re-fetch and fail closed on ambiguity. Do not claim transactional multi-writer guarantees until validated with two concurrent clients.

## 6. Suggested delivery

P0a: Rust abstraction, SQLite baseline/journal, deterministic planner and in-memory fake backend tests.
P0b: S3 adapter (simple API; include MinIO integration), first sync preview, manual two-way sync, conflict copies and safe deletes.
P0c: Yandex REST + OAuth, account settings, sync polling, robustness and real two-client tests.
P0d: Google Drive v3 + PKCE app folder, stable ID mapping and pagination; real OAuth and two-client tests.
P1: automatic watchers + debounced sync, progress + conflict UI, large files, retry/cancel/resume, optional encryption.
P2: mobile synchronization, sharing, multi-provider mirroring if genuinely required.

Do not merge a provider into Akula main on the strength of mocked unit tests alone. Each provider needs an isolated test account/bucket + two-device real-data roundtrip and deletion/conflict verification.
