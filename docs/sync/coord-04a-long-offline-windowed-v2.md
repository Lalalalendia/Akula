# SYNC-COORD-04a — long-offline V2 verified locator windows

Status: **opt-in read-only experimental entrypoint**, no activation in normal sync, no baseline or File Gate writes.

## Why

The accepted 02e/03b V2 selective packed reader is limited to 8 missed generations (even if each generation has only one small revision) despite the canonical V1 ancestor walker supporting at most 128 historical frames. A receiver missing 20 small generations must not treat its final V1 manifest as a single new revision set: this would silently drop intermediate tombstones, offline sibling revisions and causality.

The accepted 10k MinIO #238 benchmark proved 16 SDK GET for 7 edits and 316 for 1000 edits (two missed generations), versus 21,115/22,605 full V1 GET. The initial 10k cold anchor pin itself cost 71,108 GET. These are local read-only figures with different old-blob reavailability assurances; do NOT claim a full product speedup.

## Implemented boundary

A newly named SDK **read-only** API `inspect_v2_long_offline_selective_packs_discovered` is available under opt-in `sync-s3`. The original `inspect_v2_offline_selective_packs_discovered` and explicit-hash verifier remain hard capped at 8 generations. No V1/V2 wire schema or S3 keyspace changes.

1. The existing profile-bound, previously fully verified private SQLite V1 anchor and metadata cache remain **unchanged**. No intermediate generations are promoted to another trusted baseline.
2. Validate every V1 predecessor against the exact saved anchor and the final observed S3 root/ETag via the existing canonical `walk_s3_delta_from_saved_anchor`, up to the preexisting **128-frame** limit.
3. Process the verified immutable frames from oldest to newest in **eight-generation locator windows**. Each locator is derived from the exact previously verified V1 manifest ID, strictly decoded against its reconstructed generation/root, never found through S3 LIST or accepted as an authority.
4. Reuse the SAME 02e read-only pack verifier with the explicit total-frame budget. It independently walks the whole anchored V1 chain, checks every V1 shard inventory, every V2 letter's old/new coverage, all bounded new pack JSON/blob bytes and the full revision DAG; it re-reads the final S3 current pointer and opaque ETag after downloads.
5. Keep global resource ceilings. Canonical walk max 128 frames, 32,768 inspected V1 shard references and 16 MiB manifest bytes. Verified additional V2 **letters 16 MiB**, packs **16 MiB**, newly downloaded shards **32 MiB**. Individual V2 letters/packs retain 512 KiB and 128 pack-member limits. Failure or 129 generations -> HOLD, not an automatic unsafe snapshot adoption.
6. Return only `S3CoordSelectivePacksPreview`, containing all new immutable RevisionIds and `requires_full_apply_gate=true`. The caller gets NO File Gate rights, no file replacement, no cloud delete/GC, no writer authentication, no durable sync baseline.

The eight-generation windows are currently **in-process iteration groups, not batched or concurrent S3 GETs and not restart-resumable windows**. They do not reduce network round trips by themselves. The 128-generation limit is a maximum *subject to* the aggregate byte and manifest/shard budgets; a large vault may hit HOLD sooner. The reader still makes **two bounded V1 walks** (one for letter locator discovery, one for independent 02e pack validation) because no trusted continuation handoff exists yet. Optimizing these redundant metadata GETs is a separate measured PR, not a reason to weaken history verification. This is not a multi-window durable transaction: windows are only bounded enumeration groups under ONE prior anchor. It does not bypass host custody or change the current publisher.

## Real two-client MinIO acceptance

Ignored disposable test `live_minio_v2_twenty_offline_generations_locator_windows_missing_letter_hold`:

- Fully pin genesis with one Markdown revision and persist a profile-bound SQLite anchor; create 20 individual CAS-published V1 successors with distinct immutable revision JSON, blobs (or a tombstone), V2 packs, letters and manifest-ID locators. All 20 revisions preserve the same original parent as siblings; one is a tombstone.
- Deliberately publish generation 10 before its immutable locator. The opt-in long reader MUST refuse even when later V1 roots are visible. After making only that missing locator immutable, long reader must verify all 20 generations, all exact new IDs and 21 DAG records.
- The old 8-generation entrypoint must still refuse the same long chain; the original SQLite anchor must remain at genesis after the long preview.

**Potential later cases:** 100 generations with small pack contents; 129-generation refusal; tampered middle V2 member, missing tombstone, changed final ETag; WAN 30/100/200ms RTT; metrics for local SQLite cold cache and GPU-independent UI. A 100-generation test is not yet a measured WAN result.

## Remaining release blockers

No actual local apply or confirmed baseline, no old blob current-availability re-proof, no authenticated publisher or anti-rollback, no installed-host package proof, no E2EE/automated file replacement, and the V1 32,768-revision-reference ceiling remains. Do not enable background/user sync or delete old immutable material on the strength of this preview.

**Follow-up:** only after this PR is accepted, reduce duplicate walker round-trips using a single bounded verified-history handoff, then measure real WAN and large multi-generation fixture. The locally installed host custody P0 proceeds separately.
