# SYNC-DELTA-01e — durable, profile-bound S3 remote observation anchor

**Status:** opt-in Rust module, does **not** enable automatic two-way S3 sync, local File Gate writes, file deletion, GC, or per-document `ConfirmedBaseline` advancement. Maintains canonical S3ManifestV1; no new S3 object key or remote wire protocol.

Prerequisites: merged [#149](https://github.com/Lalalalendia/Akula/pull/149) canonical S3 manifests, [#155](https://github.com/Lalalalendia/Akula/pull/155) fully pinned S3 revision snapshots and [#160](https://github.com/Lalalalendia/Akula/pull/160) read-only ancestor walker. Source: [Notion SYNC-DELTA-01](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360), [Tela](https://telawiki.com/spaces/456/pages/7501/sync-delta-01-shard-reuse).

## Distinguish three facts

1. **Remote observation anchor:** a durably recorded S3 head (generation, manifest ID/hash, ETag) and exact, canonical immutable manifest bytes for one storage profile and vault. This PR stores only this fact, subject to verified upstream metadata. Read-only network delta discovery may use it later.
2. **Fully verified remote revision snapshot:** obtained through existing `pin_revision_snapshot`, which validates manifest, shards, revision DAG, referenced blobs and required ancestry. Its immutable `snapshot_id` must match the remote head. This must be supplied before recording a remote anchor; a raw S3 head alone cannot create a checkpoint.
3. **Per-document accepted baseline:** the local SQLite `ConfirmedBaseline` and File Gate evidence proving the contents were safely applied to the user's vault. This PR does not modify, confirm or infer that state. Never label remote head equality as “sync complete”.

## Scope and durable storage

`S3RemoteAnchorScope::new(profile_id, verified_account_binding, &S3ConnectionProfile, &S3Keyspace)` validates and binds:
- Non-secret stable profile ID and externally verified **non-secret** account/deployment binding.
- Exact region, bucket, endpoint including scheme/port (or standard AWS regional endpoint), force-path-style setting, plus vault-scoped S3 `current_head_key` (includes prefix and VaultId).
- BLAKE3 fingerprint, domain separated from content hashes, stored alongside the record. Any accidental reuse of the same profile ID and vault ID against another endpoint/bucket/account/root fails **ScopeChanged**; never silently reboots as new empty remote.
- Account binding is an **input assertion**. The code does **not** fetch AWS STS caller identity or independently authenticate the author; calling it “verified” requires a separate identity/credential provider. Changes to the credential principal with the same claimed account binding are **not** detected.
- No credentials, user paths or note contents are persisted.

Use caller-selected **private app-data**, not the user vault, for this dedicated SQLite file. Schema `s3_remote_anchor_v1` stores canonical head bytes, verified manifest bytes, ETag, fingerprint and a domain-separated integrity digest. SQLite runs WAL, `synchronous=FULL`, `foreign_keys=ON`, busy timeout and `BEGIN IMMEDIATE` per publication to serialize concurrent writers.

The receipt digest detects accidental corruption and undetected accidental field changes. It is **not a MAC**: malicious local DB replacement/rewind is outside this stage's protection. Do not reuse this state as a trusted anti-rollback root until independent authenticated monotonic trust is available.

## Record/update safety

`record_verified_remote(scope, &verified_head, &pinned_snapshot, &[raw_manifests_newest_first])` is inert with respect to user files and remote S3:
- Verify vault-scoped canonical `S3HeadPointerV1`, exact `S3ManifestV1` hash, root generation, predecessor identity and `pinned_snapshot.snapshot_id`/revision count.
- New profiles must supply a complete ancestor chain to genesis. Existing profiles must supply all immutable manifests from candidate head to the **persisted old anchor**. Validate through the already-merged `inspect_s3_manifest_delta`; do not trust wall-clock mtime, S3 listing or an unverified snapshot cursor.
- Same pointer/ETag/bytes => idempotent no-op. Same pointer with changed ETag or mutable-looking manifest => hard hold; lower generation or same-generation fork => rollback/fork refusal.
- Missing or expired ancestor chain => **IncompleteAncestry** / **VerifiedSnapshotRequired** (no deletion, no “empty remote” fallback).
- SQLite `BEGIN IMMEDIATE`, compare prior receipt, write canonical bytes and commit atomically. On process restart, load validates receipt and re-decodes head+manifest; corrupt rows fail closed.

**Important integration gate:** the current `S3 Delta Walker` returns a read-only `S3DeltaWalkPreview` rather than retaining the entire ancestor-chain byte vector. Therefore this standalone journal does not yet auto-record after a walker result. The next PR must provide verified chain bytes or an opaque immutable proof token, without trusting an unbound caller-provided checkpoint. A separate end-to-end MinIO proof is necessary before activation.

## Acceptance tests

- Private SQLite WAL commit survives reopen; repeated identical observation is idempotent.
- Two offline generations cannot advance without both exact immutable manifest bytes; with intact chain, generation increases.
- Accidental reuse of profile/vault against new bucket/account/endpoint/prefix refuses stale checkpoint.
- Rollback/same-generation fork, changed provider ETag for identical pointer, mismatched pinned snapshot, corrupted SQLite contents, invalid scope/endpoint and stale second-process writer all fail closed.
- Only test-compiled, empty pinned-revision fixtures are used by unit tests. Production `S3PinnedRevisionSnapshot` remains constructed through canonical verification.
- `sync-s3` test suites, default core and OS policies, MinIO acceptance and Main PR gate must succeed on exact final head.
- No benchmark conclusions at this step. Existing 256 × 128 = 32,768 revision-ref ceiling persists.

## Next

Bound durable anchor scope to actual authenticated connection/identity evidence and connect it to `walk_s3_manifest_delta` without copying unchecked bytes. Verify every candidate shard/revision/blob through existing `S3VerifiedRevisionMetadataStore` and File Gate before any accepted local document baseline; then run A/B on 10k Markdown notes (0/7/1000 edits, scattered IDs and injected RTT).
