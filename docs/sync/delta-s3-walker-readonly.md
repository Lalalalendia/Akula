# SYNC-DELTA-01d — read-only S3 manifest ancestor walker

**Stage:** opt-in Rust `sync-s3` observation, **not** an automatically activated synchronizer. This continues [SYNC-DELTA-01](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360) after merged PRs [#143](https://github.com/Lalalalendia/Akula/pull/143), [#148](https://github.com/Lalalalendia/Akula/pull/148), [#149](https://github.com/Lalalalendia/Akula/pull/149) and [#156](https://github.com/Lalalalendia/Akula/pull/156).

## Protocol discovery without a new wire format

Two clients do not directly negotiate over S3; each reads the mutable `refs/current.json`, then immutable `S3ManifestV1` objects referenced by ID and BLAKE3. A durable verified local checkpoint identifies the previously accepted ancestor. The new `walk_s3_manifest_delta`:

1. Read vault-scoped S3 head via `get_bounded` (HEAD → conditional GET → HEAD).
2. If `inspect_s3_manifest_delta` finds the same **previously confirmed** head, do not fetch an immutable manifest. This is **not** proof that local edits are absent.
3. Otherwise repeatedly follow `NeedOlderManifest`, fetching the exact immutable manifest key and checking declared BLAKE3, strict `S3ManifestV1` validation and predecessor generations. Avoid untrusted `ListObjectsV2` for snapshot discovery.
4. Stop at verified checkpoint/genesis; return only read-only candidate `S3ManifestShardRefV1` metadata. Checkpoint bytes must be trusted **and associated with the same account/bucket/prefix/vault**, which is a separate prerequisite; the struct alone cannot establish that.
5. Re-fetch mutable head and compare decoded pointer, exact JSON bytes and provider ETag. If different, reject the candidate plan and replan. No automatic conflict resolution or baseline advancement.
6. If a required ancestor is missing, corrupted, inaccessible or outside length budgets, fail closed. Full-snapshot fallback is an explicit, separate *verified* operation, not a synthesized empty remote.

## Network and memory bounds

- Each manifest is bounded by canonical `MAX_MANIFEST_BYTES` (512 KiB).
- Maximum ancestor depth and aggregate shard references come from `S3ManifestDeltaLimits` (default 128 frames / 32,768 cumulative shard references).
- Additional walker-wide immutable-manifest byte budget = 16 MiB.
- No recursion or open-ended S3 listing; no `PUT`, `DELETE`, `FileGate` changes or SQLite journal writes.
- Each object `get_bounded` **itself** consists of HEAD, GET(If-Match), HEAD, so a no-change observation currently performs two bounded head reads (up to six S3 requests), not a single network packet. Do not claim O(1) HTTP latency improvement until this is measured.
- Strict shard snapshot capacity remains `MAX_SHARDS=256` × `MAX_SHARD_ENTRIES=128` = **32,768 revision references**. A 100,000-revision experiment requires a separately versioned/indexed expansion and migrations.

## Fault model

- Offline laptop for two generations: exact immutable lookup newest → parent → confirmed checkpoint; no reliance on wall clock.
- Concurrent head CAS/ETag rotation: final re-read detects an ordinary race; a same-byte/ETag ABA cannot be distinguished by this probe alone. A future authenticated monotonic authority is needed for strong anti-rollback.
- Damaged predecessor digest or unrecognized schema: fail closed. S3 `NotFound`, 403 and other unavailable states never authorize local deletions.
- Repacked/superseded shards: hints only. Require **full canonical revision retention verification**; a missing shard ID alone never means removed documents.
- BLAKE3 proves internal linkage *relative to received head*, not writer authorization, E2EE integrity or complete blob accessibility. Only canonical `pin_revision_snapshot` verifies referenced revision JSON and blobs. The delta walker does not call it because that would for now re-fetch the entire contents and obscure discovery measurements.
- Read-only preview must never be used directly to execute `StageDownload`, apply tombstones or update the confirmed baseline.

## Acceptance and follow-up

1. Unit tests with injected S3 reads: unchanged head, offline 2-gen, genesis, missing/bad ancestor, immutable bytes/length consistency, foreign vault, changing root and ETag, depth limit.
2. **Disposable localhost MinIO:** test `live_minio_delta_walker_three_generations` publishes three small immutable manifest objects and versioned head CAS in its own throwaway bucket, then proves one head-only no-change observation and two ancestor downloads with the official AWS Rust SDK.
3. Future performance A/B: 0/7/1000 edits in ~10k notes; report application-level reads, raw HEAD/GET count, wall time, p95, bytes and errors, including realistic RTT and 7 random scattered UUIDs. Do not label unit fixtures as measurements.
4. Persistent, storage-profile-bound verified checkpoint, verified shard/payload handoff and native File Gate execution belong to separate reviewed stages. Restore after too-old checkpoint by a fully verified pinned snapshot, not a naive ListObjectsV2 scan.

See also [Tela](https://telawiki.com/spaces/456/pages/7501/sync-delta-01-shard-reuse), [canonical ADR](https://github.com/Lalalalendia/Akula/blob/main/docs/sync/delta-shard-diff-experiment.md) and the [PR00–11 roadmap](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11).