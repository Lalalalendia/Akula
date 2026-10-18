# SYNC-COORD-02d — real MinIO read-only equivalence A/B

**Status:** PR experiment. No new production S3 writer, File Gate permissions, manifest schema, CAS, E2EE, or trusted baseline advancement.

## Goal

Measure the actual cost of the conservative 02c per-generation full V1 pin against the already implemented anchored selective V1 reader, without assuming that the original 2-GET toy Merkle fixture was production sync. The test uses an identical remote state and the same durable, profile-bound SQLite anchor for three independent read-only checks:

1. **Conservative V2 chain:** fully pin each missed V1 generation, then verify the matching immutable V2 letters/packs and all member hashes/DAG.
2. **Selective V1:** verify the immutable manifest lineage, intermediate tombstone/sibling retention, changed shards/revision JSON/new content, and final head+ETag; reuse prior fully verified immutable metadata and blobs.
3. **Full current V1:** re-read and verify the complete final V1 snapshot as the independent full-availability reference.

These are **not equivalent authorizations**. The test requires identical sorted new RevisionId membership between 1 and 2 and identical final complete RevisionId membership between 1/2 and 3. The selective read deliberately does not re-prove that all historic blobs still exist, so it must **never** serve as a File Gate apply certificate or ConfirmedBaseline.

## Real MinIO acceptance

Extends the existing ignored two-client offline test `live_minio_v2_two_offline_generations_retains_tombstone_and_rejects_gap`: genesis plus an offline content sibling in generation 2 and an independently added tombstone sibling in generation 3. The same V1 durable anchor, real S3 SDK and disposable localhost bucket back every mode. A deliberately malformed generation 4 that removes the tombstone must be rejected by **both** the V2 chain and the selective V1 reader.

The `AKULA_COORD_02D_PARITY` JSON line reports per-mode SDK GET attempts, HEAD attempts, returned bytes and elapsed microseconds. These counters do not include SDK internal retries/TCP framing or full application work. The performance comparison does not assert that the selective mode must win on a tiny fixture: the purpose is first to establish identity and safety.

## Acceptance / next step

The existing `live_minio_` GitHub Actions step runs this ignored MinIO test; no credentials or live S3 buckets are used. CI must pass on the **final head** before merge. Follow up with 10,000-note 0/7/1000-edits benchmark and WAN-latency probes, comparing actual complete verification costs and whether selective verified packs can safely substitute fresh JSON/blob reads. Do not enable user-file application before separate authentication, current-availability, encryption/capability and crash-safe File Gate gates.
