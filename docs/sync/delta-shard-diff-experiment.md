# SYNC-DELTA-01c — canonical sharded-manifest delta inspection

**Status:** read-only optimization experiment, not activated user synchronization.

This implementation continues [SYNC-DELTA-01](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360), building on merged [PR #143](https://github.com/Lalalalendia/Akula/pull/143) and [PR #148](https://github.com/Lalalalendia/Akula/pull/148).

## One authoritative cloud format

Use the canonical **SYNC-05d1 `S3ManifestV1`**, already merged as [PR #149](https://github.com/Lalalalendia/Akula/pull/149). Its root references immutable content-addressed shards (each with `shard_id`, `shard_blake3`, bounds and entry count) and carries both the immediate predecessor manifest ID and BLAKE3.

Do **not** publish another manifest schema. Earlier draft [PR #151](https://github.com/Lalalalendia/Akula/pull/151) proposed a distinct `DeltaManifestV1`; this is superseded by the present approach to avoid split-brain wire formats.

## Pure inspector

`inspect_s3_manifest_delta(keyspace, current_head, verified_checkpoint, newest_first_manifests, limits)` operates entirely in memory. No S3 requests or filesystem writes are permitted.

- Check `S3HeadPointerV1` generation/vault, current and previous immutable manifest IDs and exact BLAKE3 bytes.
- Decode **existing** `S3ManifestV1` strictly; reject malformed, oversized and foreign-vault content. Require each parent generation and expected hash to match.
- Walk newest-first, returning `NeedOlderManifest` with an exact immutable ID, expected digest and generation until a trusted checkpoint/genesis is reached.
- Stop with `RequiresVerifiedSnapshot` when the chain is too deep or a checkpoint is unrelated. A snapshot is a separate independently verified operation, **not** a synthetic deletion of missing files.
- Once ancestry is proven, compare the immutable shard references in latest vs checkpoint: **identical shard ID+all metadata → reusable; new shard ID → candidate for validated fetching**.
- Reusing a shard ID with a different digest/metadata is a hard collision/error; sharded object names are immutable.
- A checkpoint shard absent from the newer index is marked `superseded_shards` and `requires_verified_revision_retention`; it must not be interpreted as a deleted document. Resharding/repacking is normal.
- Candidate shards are just hints. The canonical S3 pinning path must still read shards, revision JSON and blobs with BLAKE3 and enforce preservation, DAG parent closure and File Gate.

No author authentication, persisted profile-bound checkpoint, rollback-proof trust root, user sync execution, CAS publication or encrypted metadata is implemented by this module.

## Complexity and performance

No changes to S3 object namespaces, no new wire required, and no per-file HEAD for remote change **discovery**. With unchanged root, no manifest bytes are needed. For changed roots, verifying N missed generations can need N bounded manifest fetches. Comparing two roots is at most 256 shard references each, excluding the cost of manifest ancestry verification. The existing `get_bounded` transport itself performs HEAD / conditional GET / HEAD.

**Critical caveat:** the current sharding algorithm can reorder chunk boundaries when inserting revisions. Seven changes among 10,000 revisions could rewrite *many* shards if a publisher rebuilds contiguous chunks from sorted UUIDs. The current inspector exposes changed/superseded shard counts but does not guarantee a seven-object network delta. A future stable-range or hash-prefix shard policy, with separate migration and security review, may be necessary.

## Tests and acceptance

- Unchanged confirmed head: no manifest fetch.
- One added shard: reuse unchanged references and return only the additional candidate.
- Multiple offline manifest generations: exact parent lookup and verified common checkpoint.
- Genesis preview: no file write authorization.
- Repacked shard: candidate fetch plus explicit retention-verification requirement.
- Immutable ID reuse with changed hash, parent corruption, checkpoint tampering, fork/rollback: fail closed.
- Caller-defined frame/shard caps cause verified-snapshot request, not partial acceptance.
- Deterministic fixture of 10,000 revisions plus seven localized inserts: 78 reused shard references, two candidate shards, one superseded shard.
- Run real 10k/100k MinIO benchmarks later with 0, 7 and 1,000 changes, including adversarial UUID distributions, mixed file sizes, RTT, CPU/RAM and remote request count. **No speedup claimed from unit tests.**

## Integration sequence

1. Accept read-only pure inspector on current main, without introducing new schema.
2. Add an opt-in S3 read-only walker that fetches `NeedOlderManifest`, ensures profile-bound checkpoint and rechecks current head after the entire traversal.
3. Connect candidate shard references only to the existing pinned verification pipeline.
4. Compare against full S3 snapshot discovery under real MinIO and simulated latency. Investigate stable sharding only if measured savings warrant it.
5. No local writes/automatic sync before canonical stage SYNC-04/05e gates.
