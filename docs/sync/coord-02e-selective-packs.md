# SYNC-COORD-02e — selective V1-verified shard inventory + V2 pack read

**Status:** opt-in production-compiled read-only SDK inspection, not the application's active S3 sync mode or user-file apply path. Follows merged #229 (conservative offline chain) and #233 (real MinIO three-mode comparison).

## Problem and limited solution

#233's two-offline-generation MinIO run proved equal added RevisionIds for three modes, but the conservative chain spent **24 GET + 48 HEAD** versus **12 GET + 24 HEAD** for existing selective V1 (three total revisions). The full V1 pin was needed to prove historical content availability, but it doubled work when the purpose was merely read-only change discovery.

The new `inspect_v2_offline_selective_packs_against_pinned_v1` is a *fourth* method. It takes the same exact S3 SDK profile, namespace, full-pin-constructed old snapshot, complete previous revision-metadata cache, **durable private SQLite remote anchor**, and explicitly supplied ordered V2-letter digests. It:

1. Rejects unmatched profile, foreign vault, forged/duplicate letter locator, unsupported count, wrong prior cache/snapshot or invalid canonical cached old shard metadata before network access.
2. Reuses the canonical `walk_s3_delta_from_saved_anchor` history verifier across all missed generations, up to **8** linked immutable manifests and a stable observed S3 pointer+ETag.
3. Reconstructs the old V1 shard bytes *canonically* from the previous completely pinned revision refs, re-hashing each old shard descriptor. For each later generation, reuses a shard only when **both immutable ID and BLAKE3 match**; otherwise fetches each novel shard with bounded HEAD → If-Match GET → HEAD and checks exact schema, count, ID ranges, content hash and global ref order. Rejects shard rewrites and a cumulative changed-shard download above **32 MiB**.
4. Verifies each V2 canonical content-addressed letter against complete **V1 shard-derived** old/new revision inventories, requiring preservation of every old UUID, conflicting sibling and tombstone. One letter per generation; max **1024 additions per letter**.
5. Downloads newly added revision JSON and blobs through the **existing verified V2 packs** (max 128 members/pack, 512 KiB/pack, 16 MiB across the offline chain). The shared pack decoder validates JSON identity, blob digest/length, precise changed UUID membership and complete in-memory DAG. No new individual JSON/blob GET is issued in this path.
6. Checks the final mutable V1 head **and ETag again** after all reads. Returns only a separate `S3CoordSelectivePacksPreview` with counts, IDs and `requires_full_apply_gate=true`.

The public `verify_pinned_v1_coverage` remains unchanged and continues to accept only full-pin snapshot types. A **crate-private** shard-inventory advisory comparator uses the same arithmetic without presenting shard-only inventories as full-pin receipts.

## Real tests and safety

Extend the real localhost MinIO two-client fixture from #233: compare the new selective packs preview against the conservative V2 full chain, selective V1 and complete final V1 pin, with identical revision identity sets. Include offline sibling + tombstone retention, missing/reversed letter rejection and gen4 history truncation rejection. Emit `AKULA_COORD_02E_PARITY` counts of SDK GET, HEAD, returned bytes and elapsed microseconds.

Pure Rust checks reject old shard cache with forged JSON hash or UUID and strictly cap bytes/generations. Mandatory PR CI must run with `--features sync-s3`; the existing `live_minio_` integration lane exercises the actual SDK/MinIO path.

**Security boundaries deliberately unchanged:** no proof that *old* previously verified S3 blobs remain available today, no writer signature/authenticated identity, no encrypted metadata/privacy claim, no local filesystem access, no CAS publishing, GC/deletion or baseline advancement. Never substitute this new advisory result for a full File Gate apply gate. A measured faster read is not a safe complete sync.

## Follow-up

Capture exact-head CI MinIO `AKULA_COORD_02E_PARITY` measurements and compare against #233. If parity and negative tests pass, measure 10,000-note workloads with 0/7/1000 additions and repeated offline generations, accounting for actual shard reshuffling/write amplification and WAN RTT; avoid making performance claims from tiny fixtures. Writer auth, capabilities/E2EE, current old-blob availability and crash-safe native apply remain independently required.
