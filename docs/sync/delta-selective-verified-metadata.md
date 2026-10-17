# SYNC-DELTA-01g — selective verified revision/blob *read-only* preview

**Implementation scope:** opt-in `sync-s3`; no new remote wire schema, no native file writes, no S3 PUT/DELETE/GC in the production API, no document ConfirmedBaseline advancement. Reuse canonical `S3ManifestV1`, the [anchored walker](https://github.com/Lalalalendia/Akula/pull/169), the verified [S3 metadata cache](https://github.com/Lalalalendia/Akula/pull/155) and the immutable object reads implemented earlier.

## Why

Currently `read_verified_manifest`, `read_verified_head` and `pin_revision_snapshot` walk *all* revision JSON and referenced content blobs, including previously checked immutable data. This is correct for a full pin, but makes every ordinary recheck potentially depend on total vault size and turns a 7-file edit into hundreds/thousands of S3 reads. We must separate **delta discovery / new data verification** from **fresh full availability pinning / permission to apply local changes**.

## Implemented API

`AwsS3ProbeClient::inspect_selective_s3_revision_changes(scope, journal, previously_pinned_cache, metadata_budget, limits)`:

1. **Before any S3 I/O:** require actual SDK profile equals scope (account claim is still externally verified), reload the private SQLite remote anchor, and check that the old complete DAG cache is bound to *exactly* the anchor's VaultId and `manifest_id:blake3`. A client-created arbitrary checkpoint cannot bypass these checks.
2. Use `walk_s3_delta_from_saved_anchor` to verify the current pointer / predecessor hashes with a re-read of mutable head. When identical, return an advisory no-change report without downloading immutable manifests, JSON or blobs. Local pending edits are still independent.
3. For an advanced pointer, require **exactly one verified successor generation** from the trusted checkpoint. More missed generations currently return `VerifiedFullHistoryRequired`, never partial success. This conservative constraint ensures intermediate remote siblings or tombstones cannot be silently omitted without per-generation revision-retention proof.
4. Fetch the latest canonical root by immutable manifest ID / BLAKE3, check generation and exact predecessor. Compare its shard references against the canonical previously pinned manifest in SQLite.
5. **Unchanged shard ID + full descriptor** → reuse its already checked revision JSON from the old fully pinned in-memory cache, and validate exact range and count. **New/repacked shard ID** → bounded GET, exact BLAKE3, strict decode and range validation. An existing revision UUID can be reused inside a new shard only if *all* metadata (hash, document ID, blob length) match the old verified record.
6. **New revision** → bounded GET + BLAKE3 + checked JSON semantics; **new unique content blob** → existing streaming `digest_stream_bounded` with BLAKE3 and exact length, once per unique new hash. Tombstones never trigger blob reads; they remain logical revisions.
7. Require **every** old revision/tombstone/sibling to be preserved; reject dropped revision, duplicate identity, same UUID with different data, changed blob length, broken or cross-document DAG parents. Build/check the full DAG *in memory* with the existing `S3VerifiedRevisionMetadataStore` validation, but deliberately do **not** expose it as a fully pinned or writable RemoteStore.
8. Re-read the mutable head (pointer and provider ETag) after all requests. Changed head → Hold and replan. Return only `S3SelectiveRevisionPreview`: counts, new revision IDs, snapshot ID and an explicit `requires_full_apply_gate=true`.

## Critical trust and availability boundary

- Reuse proves **previously verified** immutable bytes match a later signed-by-hash canonical reference, **not** that old S3 blobs still exist *now*. The module does not re-request unchanged old blobs; S3 operator deletes, compromised account, GC or lost availability can invalidate the remote. It is **not** a fresh full pin for applying user file changes or GC.
- It never returns a `S3PinnedRevisionSnapshot`, a complete `RemoteStore`, or a File Gate write receipt. It does **not** persist the next anchor or decide whether old or new sibling wins. A future separately reviewed selective **availability/retention contract**, cryptographic writer authentication and File Gate authorization are needed before this result can drive actual user file changes.
- The old cache must be generated from a complete full pin via `materialize_verified_revision_metadata_store` and kept in memory (not persisted across restarts in this stage). After process restart, the **SQLite anchor persists**, but the old cached metadata JSON may need an expensive full re-pin; durable verified cache is future work.
- `S3ManifestV1` still has a maximum of 256 shards × 128 revision entries = **32,768** total references. Stable shard distribution matters: a sorted UUID re-pack can force many changed shards for seven scattered revisions.
- The operation is read-only and network bounded. S3's `get_bounded` runs HEAD/conditional GET/HEAD; unchanged-head path observes head twice, so this optimization primarily reduces immutable-object reads rather than necessarily reducing connection handshake overhead.

## Tests and acceptance

- Unit: exact verified old revision reference reuse, changed UUID rejection, tombstone no-blob semantics, retention of two offline sibling revisions, inconsistent lengths, missing-parent DAG hold.
- **Real disposable localhost MinIO** `live_minio_selective_one_edit_reuses_unchanged_revision_and_blob`: publish and fully pin genesis (1 revision + content blob), persist/reopen SQLite anchor, publish second manifest reusing original shard plus one new revision; first omit new blob and confirm explicit `NotFound`, then upload new blob and confirm selective read succeeds with 1 reused shard/old JSON and 1 new shard/JSON/blob. Check remote anchor remains unchanged.
- Default Rust and `sync-s3` tests, Linux/macOS/Windows File Gate protections and Main PR gate on exact PR SHA. No merging until required checks pass.
- **Later benchmark:** real S3 request counts, bytes, CPU and wall time for 10,000 notes with 0, 7 and 1,000 changes and random/spatially clustered revision IDs. Compare against full `pin_revision_snapshot` and measure exposure to RTT. Tests above are functional, **not proof of a performance gain**.

Related work: [Notion SYNC-DELTA-01](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360), [Tela SYNC-DELTA-01](https://telawiki.com/spaces/456/pages/7501/sync-delta-01-shard-reuse).
