# SYNC-COORD-02c — bounded, conservative multi-generation V2 sidecar replay

**Status:** isolated PR, CI not yet accepted. This adds only a read-only, advisory, real AWS-SDK S3 path and does not change the existing production V1 sync wire schema, root publication, File Gate, user files or deletion/GC. It extends the merged [V2 direct-generation reader #213](https://github.com/Lalalalendia/Akula/pull/213) while preserving the [all-generations retention invariant #197](https://github.com/Lalalalendia/Akula/pull/197).

## Problem

An offline client may miss generation 2, 3, ... 8 while old and current S3 roots both look self-consistent. The direct-generation read-only sidecar of #213 refuses this scenario rather than risk silently dropping an intermediate tombstone or offline conflict sibling. The toy pack benchmark [#201](https://github.com/Lalalalendia/Akula/pull/201) is not sufficient to authorize a fast multi-generation replay.

## Implementation

- Add `AwsS3ProbeClient::inspect_v2_offline_chain_against_pinned_v1(scope,journal,previous_pinned,previous_metadata,letters_oldest_first)`; returns only `S3CoordOfflineChainPreview` with generation range, exact added RevisionIds, checked pack bytes/count and `requires_full_apply_gate=true`.
- Require the real AWS SDK profile, bucket, region, endpoint, path style, namespace and VaultId to match a saved **profile-bound SQLite S3 remote anchor** before any network I/O. Compare the previous canonical V1 pinned snapshot and fully verified JSON metadata to the exact saved anchor; no forged array or caller-provided floating V1 root is accepted.
- Reject empty, duplicate or malformed letter locators and strictly cap replay to **8 contiguous generations**. Call `walk_s3_delta_from_saved_anchor` with strict 8 frame / 2048 cumulative shard-ref appearances, checked immutable predecessor chain and stable mutable current-head/ETag observations.
- For every missed canonical `S3ManifestV1` (oldest-first), derive its V1 head pointer from BLAKE3-linked canonical manifest bytes, **independently full-pin** that historical immutable manifest with the existing `read_verified_manifest` pipeline. This validates **all shards, all revision JSON, all content blobs and the complete DAG** at every generation. Build a typed `S3PinnedRevisionSnapshot` internally, not from untrusted letter lists.
- Read the matching immutable `S3DeltaEnvelopeV2` via bounded **HEAD → If-Match GET → HEAD** and verify exact whole-letter BLAKE3/canonical codec. Compare the letter against fully pinned old/new V1 snapshots with `verify_pinned_v1_coverage`. This requires every previous reference (including tombstones and both sibling conflict branches) to survive bit-for-bit; its changed-ID list must be the exact new set.
- Download each packed object via the existing bounded SDK and share the **same `verify_letter_packs`** as the direct generation reader: whole pack BLAKE3 and byte length; every revision JSON, document identity, blob hash/length, tombstone semantics; complete in-memory DAG with all historical parents and no dropped/conflicting UUID. Pack payload cumulative cap **16 MiB** across the whole replay, checked against **declared pack bytes before downloading**.
- Final V1 root pointer and ETag must match the stable walker observation after all full pins/pack downloads. Every missing/rewritten ancestor, wrong letter order/count, unreadable/oversized pack, forged root or changed final ETag is a **hard hold**. The previously saved SQLite remote anchor is never advanced by this read-only operation.
- The snapshot helper is private to S3 transport and invokes the full V1 verifier itself. No independently trusted writer key/signature, V2 CAS writer, GCM/E2EE, filesystem write, delete, GC, automatic conflict winner or `ConfirmedBaseline` in this PR.

**Important measurement limitation:** full historical V1 re-pinning is intentionally expensive and will not produce a 2/9 GET production sync speedup. It is the conservative *correctness gate*. A subsequent bounded incremental pack path can replace redundant verified reads only after exact content/metadata equivalence and lossless crash-safety are proven.

## Tests / acceptance

- Pure Rust boundary test asserts the strict 8-generation/16-MiB limits.
- Ignored integration test `live_minio_v2_two_offline_generations_retains_tombstone_and_rejects_gap` uses two independent official AWS Rust SDK clients and a disposable localhost MinIO bucket: fully pin Genesis + persist SQLite anchor, post gen2 content sibling, gen3 tombstone sibling, independently verify both V2 letters/packs and full DAG; reject missing/reversed letter, then publish a valid-looking gen4 that omits gen3 tombstone and prove fail-closed with unchanged old durable SQLite anchor.
- Existing direct [#213](https://github.com/Lalalalendia/Akula/pull/213) sidecar tests must still pass after sharing the unchanged member verifier.
- `.github/workflows/sync-s3-minio.yml` explicitly runs fast `coord_chain::tests`; its existing `live_minio_` ignored test job runs the two-client scenario against real local MinIO.
- Exact final PR SHA requires Rust/Application, real MinIO, Docs, Windows/macOS/Linux pinned-read and protected-original checks before merge. No claim of passing until those workflows complete.

## Next boundary

SYNC-COORD-02d should measure V1 re-pin cost versus a **bounded, exact-equivalence selective packed metadata path** tied to same durable V1 root and complete per-generation retention. That performance work cannot weaken writer authentication, E2EE/capability negotiation or full File Gate apply gate. Never compare a read-only advisory receipt with fully protected remote/local publication as though they had equal authority.
