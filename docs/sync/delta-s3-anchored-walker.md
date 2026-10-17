# SYNC-DELTA-01f — binding a persisted remote anchor to real S3 reads

**Status:** opt-in read-only implementation; not full user file sync. Reuses canonical `S3ManifestV1` and immutable ancestor walker; does not introduce a new remote wire schema, S3 key, native file writer or cloud publisher.

## Dependencies

- [#165](https://github.com/Lalalalendia/Akula/pull/165) — durable SQLite **remote observation** anchor, saved only after canonical remote pin/verification. Never treated as an applied per-document ConfirmedBaseline.
- [#160](https://github.com/Lalalalendia/Akula/pull/160) — bounded, read-only immutable-manifest chain walker over S3.
- [#155](https://github.com/Lalalalendia/Akula/pull/155) — existing fully verified read-only remote revision snapshot, shards, blobs and revision DAG.
- [SYNC-DELTA-01 in Notion](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360) and [Tela](https://telawiki.com/spaces/456/pages/7501/sync-delta-01-shard-reuse).

## What is implemented

`walk_s3_delta_from_saved_anchor(client, scope, journal, limits)`:

1. Checks that the **actual SDK client** configuration (bucket, region, endpoint URL, path-style flag and S3 vault keyspace) exactly matches the externally asserted `S3RemoteAnchorScope`. The S3 client retains its validated, non-secret profile for this purpose.
2. Loads and revalidates canonical prior S3 head and immutable root-manifest bytes from the private SQLite journal. Corrupt record, changed profile or absent anchor → explicit failure **before S3 network I/O**.
3. Calls the existing bounded `S3DeltaWalker`, with the exact persisted predecessor checkpoint. Walker obtains the mutable head, traverses immutable ancestors by ID and BLAKE3, and re-observes head and ETag. Never uses S3 LIST as a point-in-time source of truth.
4. If pointer bytes are unchanged but remote ETag changed relative to persisted anchor, fails closed (`HeadRewritten`) rather than silently accepting a head rewritten with the same JSON.
5. Returns read-only `S3DeltaWalkPreview`; **does not** modify the SQLite anchor, a document ConfirmedBaseline, local files, conflict branches, S3 objects or provider CAS head. Any candidate shard is still only a fetch hint until pinned and verified by the canonical pipeline.

On first run, `NoVerifiedAnchor` is returned, not `Genesis` as an automatically authorized empty vault. An explicit separate bootstrap through existing `pin_revision_snapshot` and durable anchor recording is required.

## Critical security assumptions

- `S3RemoteAnchorScope::new` accepts `verified_account_binding` **as a caller claim**. The new adapter checks that actual connection configuration matches what was persisted, but **cannot** independently confirm the AWS principal / MinIO deployment identity or prevent a credential with equal profile metadata from pointing to a different writer.
- Stored local SQLite is **not MAC-protected** and can be replaced by a hostile local actor. BLAKE3 detects accidental corruption; robust anti-rollback requires a trusted external monotonic binding.
- A successful read-only preview does **not** prove that local documents were applied, nor guarantee availability of every referenced blob. `S3VerifiedRevisionMetadataStore` and File Gate remain separate gates.
- The initial SQLite anchor is recorded by prior code only after a trusted upstream `S3VerifiedHead`, `S3PinnedRevisionSnapshot` and complete canonical ancestor chain. This PR adds **no** automatic anchor updates. Only an already saved anchor can be used to resume.
- A concurrent writer could modify the mutable S3 head **after** the final read. Future publication must always use a fresh provider-enforced CAS.

## Acceptance tests

- Cold restart: stored anchor allows unchanged head detection with **zero immutable manifest downloads** (but two bounded head reads).
- Two offline generations: exact BLAKE3-linked ancestor GETs, while SQLite remains on its earlier confirmed observation.
- Absence of anchor, corrupted SQLite, changed bucket/endpoint/vault: error before any network request.
- Identical pointer reissued with altered ETag: explicit hold; rolled-back remote head and missing ancestor: hold without advancing local SQLite.
- Expired history depth: `RequiresVerifiedSnapshot`, not fabricated deletion/silent empty remote.
- In existing **disposable localhost MinIO** integration test, publish three canonical heads via CAS, pin root1 through existing verifier, persist a remote SQLite anchor, reopen DB, prove unchanged then prove two-gen traversal via real AWS SDK. The test never touches a user's vault.
- Check in CI: default + `sync-s3` Rust unit suites, `s3_anchored_delta::tests`, ignored `live_minio_` integration, Windows/macOS/Linux file protection and Main PR gate. Only exact final-head green permits merge.

## Next

For a truly useful rapid sync, use this anchored preview to select changed shards **and** verify them through the already existing pinning pipeline with guaranteed revision retention, without refetching every old blob. Benchmark actual S3 requests and wall time for 10,000 documents at 0/7/1,000 changes. Consider stabilized hash-range sharding if insertion causes large shard churn. Only enable file application after independent SYNC-04 and SYNC-05e acceptance.
