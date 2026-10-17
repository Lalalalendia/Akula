# SYNC-DELTA-01k — bounded verified retention across multiple offline S3 generations

**Status:** implementation in separate PR, pending all CI. This adds a read-only per-generation metadata history proof to the existing, canonical S3/MinIO delta protocol. No second cloud schema or publisher, no native File Gate write access and no head/anchor advancement.

## Motivation: latest-vs-baseline is not enough

Earlier [#175](https://github.com/Lalalalendia/Akula/pull/175) verified a latest manifest against a previously pinned one, but deliberately refused **more than one missed generation**, because a tombstone or concurrent offline sibling could have appeared in an intermediate manifest and then vanished before the final root. A latest head alone can have a complete valid DAG **even after intermediate history was silently discarded**. Correct behavior is to verify continuity of all revision IDs and their exact immutable metadata at **every** generation.

## Algorithm

`walk_s3_delta_from_saved_anchor` still first requires the exact **durably verified, profile-bound S3 observation anchor**; checks real connected SDK bucket, endpoint, region, path style, vault/prefix and pointer+ETag. Its network-bounded walker independently verifies the BLAKE3 predecessor chain and stable mutable head before returning advisory frames.

Rather than downloading those already verified manifests twice, the walker now retains its **newest-first immutable manifest bytes** as an internal-only `verified_manifest_frames` member. This vector remains bounded by existing walker limits: 128 frames, 16MiB cumulative raw immutable root bytes and at most 32,768 shard-ref appearances.

For **one missed generation**, the existing unchanged fast path is retained. For **2..128 missed generations**, the new `verify_offline_retention` gate executes **before any final revision-blob application or advisory result**:

1. Reconstruct exact existing shard membership from the previously fully pinned metadata view (or the disk hints recovered under exact S3 anchor) and verify count and UUID ranges against the canonical anchor manifest.
2. Walk all missed immutable roots from oldest to newest; independently check canonical decode, generation increments, exact previous root ID/BLAKE3 and final manifest ID/BLAKE3 against the observed remote pointer.
3. For each generation, reuse immediate predecessor shard memberships **only if the entire immutable shard descriptor is identical**. For changed/new shards, issue bounded S3 GET with HEAD/If-Match/HEAD checks, verify declared BLAKE3 and canonical schema, and compare exact `count/first_revision/last_revision`.
4. Track seen immutable shard IDs across the whole chain. Same ID associated with a changed descriptor is a **hard Hold**, not a new legitimate shard.
5. Construct the full revision-ref map for that generation, reject duplicates and count mismatches. **Every previous generation's revision ID must still exist with the exact identical document ID, JSON hash and blob length.** This preserves intermediate tombstones, sibling conflict branches and revisions even if the latest manifest could otherwise form a valid DAG.
6. Maintain only previous/next generation shard maps and revision-ref inventories in memory. New immutable shard downloads have an explicit additional **64MiB cumulative cap**; existing frame and shard-ref limits apply. Exceeding either bound is `VerifiedFullHistoryRequired` (independent full-snapshot Hold), never an empty vault/deletion.
7. Only after the complete retention gate succeeds does the existing selective verifier fetch the **latest** changed revision JSON and new blobs, recheck BLAKE3, run full DAG closure, ensure all old refs are retained, re-observe mutable S3 head and return an **advisory `S3SelectiveRevisionPreview`**. It never persists a new anchor/baseline or writes files.

An unchanged head requires no history reads; 1-generation path remains the existing fast path and keeps the previously measured warm/cold behavior. For more offline generations, extra shard reads are intentionally traded for not dropping history. Optimizing repeated shard reads and new S3 GET counts is a separate measurable performance stage.

## Failure & safety matrix

- Intermediate tombstone or conflicting sibling omitted in a later root → hard Hold even if its final root hash, manifest schema and DAG are valid.
- Previously seen revision UUID changes `document_id`, `revision_blake3` or `blob_bytes` → immutable-revision-rewritten failure.
- Previously seen immutable shard ID appears with different descriptor/digest → immutable-shard-rewritten failure.
- Missing, tampered, oversized or inconsistent intermediate shard → explicit error/Hold; never permission to delete a file.
- Broken predecessor ID/hash, generation increment or final head mismatch → reject; mutable head CAS racing final read is still handled by existing final ETag/pointer comparison.
- More than bounded 128 generations / 16MiB root frames / 32,768 shard-ref appearances / 64MiB extra history shard downloads → full-history Hold; no unbounded memory/network walk.

## Acceptance and evidence

- New feature-gated unit suite `offline_retention::tests` with two missed generations retaining both tombstone and sibling, hidden intermediate tombstone detected, UUID rewrite detected and missing/corrupted intermediate shards rejected.
- Real disposable-localhost MinIO extension to existing `live_minio_selective_one_edit_reuses_unchanged_revision_and_blob`: fully pin genesis, store/reopen private SQLite remote anchor and metadata hints, publish generation 2 new content, generation 3 tombstone, confirm after two missed generations the selective advisory preview contains both new revisions and has exactly one new blob, **then** maliciously publish generation 4 omitting the tombstone, confirm an explicit Hold with SQLite anchor still fixed at genesis. Test writes only to disposable CI bucket, never a user vault.
- CI: `cargo test --locked -p aquilum-core --features sync-s3 offline_retention::tests`, full `live_minio_` suite, existing warmed/cold 64-note A/B smoke, Main PR gate, default Rust, protected preimages and pinned-read across Linux/macOS/Windows. Merge only on exact SHA after all required checks pass.

## Security boundaries

The remote head's S3 credentials/writer identity are still externally asserted, not cryptographically authenticated by this proof. SQLite hints and BLAKE3 receipts are not MAC/anti-rollback protected against a hostile local process. Reusing old blobs does **not** prove they remain available now on S3. Returning this preview must **never** authorize File Gate user writes, deletions, GC, CAS publication, conflict winner selection or per-document baseline advancement. A separate full current blob availability and zero-loss File Gate acceptance is mandatory before applying data.

References: [Notion SYNC-DELTA-01](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360), [Tela](https://telawiki.com/spaces/456/pages/7501/sync-delta-01-shard-reuse), [#169](https://github.com/Lalalalendia/Akula/pull/169), [#175](https://github.com/Lalalalendia/Akula/pull/175), [#188](https://github.com/Lalalalendia/Akula/pull/188).
