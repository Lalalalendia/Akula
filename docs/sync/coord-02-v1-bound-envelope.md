# SYNC-COORD-02a — V1-bound compact v2 delta envelope contract

**Stage:** independent, strict Rust metadata contract; **no live provider activation**. This follows the successful [SYNC-COORD-01 A/B](https://github.com/Lalalalendia/Akula/actions/runs/39081242570): for 10,000 synthetic Markdown notes, a compact changed-ID letter + 1 pack required 2 GET on seven edits, compared to 15 when fetching seven JSON+blob pairs separately. At 1,000 changes, 8 packs + 1 letter required 9 GET vs 2,001 individual GET. **These test-only read request counts do not describe secure end-to-end Akula sync.**

## What landed in this change

`aquilum-app/core/src/sync/s3_coord_v2.rs` defines `S3DeltaEnvelopeV2` and `S3DeltaPackRefV2`. This is a **sidecar format only**, not a new production mutable remote root. It is compiled into Core in both the default and `sync-s3` feature modes but never used to apply files, publish CAS, or mutate a cloud account. The existing strict `S3HeadPointerV1` and `S3ManifestV1` remain unchanged; unknown schemas still fail closed.

**Exact one-generation parent/new anchor:** the letter carries VaultId, exact previous/current manifest IDs + BLAKE3 digests, generation, and hashes of both *canonical V1 pointer JSON objects*, not S3 ETags. The verifier requires a direct successor, both manifest digests matching two independently supplied V1 heads, and the new manifest's exact previous ID/digest.

**Complete revision membership:** the public, `sync-s3`-gated `verify_pinned_v1_coverage(...)` accepts **only two `S3PinnedRevisionSnapshot` instances created by the full V1 verification pipeline** and first checks their snapshot IDs match the exact previous/current V1 manifest ID + BLAKE3. The private inventory comparator checks their complete revision refs (sorted by typed RevisionId). It checks each inventory against its actual verified manifest count and validates strict UUID ordering/no duplicates. It requires every previous revision to survive **with identical RevisionId, DocumentId, JSON hash and blob length**, including conflict siblings and tombstones. The difference must equal the letter's entire sorted new-ref list, byte-for-byte by typed value. Hidden missing or rewritten history is a hard error, not permission for local deletion.

**Pack layout:** each pack advertises BLAKE3 content digest, bounded byte length, first revision index and member count. Ranges must be contiguous over the sorted new refs with no gaps, overlaps, reused pack digest or missing items. At most **1,024 new revision refs/letter**, **128 refs/pack**, **512 KiB/pack** and **512 KiB/letter**. Every pack's declared size must exceed the known fixed framing + declared blob bytes: truncated/clearly impossible tables are rejected even before any GET.

**Strict canonical codec:** JSON `serde(deny_unknown_fields)`, exact schema = 2, exact vault scope, lowercase 64-character BLAKE3 digests, canonical re-encoding, and size caps. Extra field, alternative JSON whitespace, foreign VaultId, malformed index, fork, rollback, non-direct ancestor, bogus root or forged UUID all fail.

**Advisory-only result:** successful verification returns `S3DeltaEnvelopeCoverageV2` with generation, added UUIDs and required pack count plus `requires_verified_pack_payloads=true` and `requires_full_apply_gate=true`. It does **not** certify pack contents exist, blob availability, writer authorization or any file write right.

## Security boundaries / why this is only 02a

1. Both `S3PinnedRevisionSnapshot` values must come from the existing **full verified remote pinning pipeline**, **not the same untrusted letter**. The only public V2 acceptance path is `verify_pinned_v1_coverage` (feature `sync-s3`), matching exact canonical snapshot IDs; raw inventory comparison is private and used in adversarial unit tests. A matched snapshot label still does not sign or authenticate the remote writer; the caller must enforce actual V1 remote scope and provenance.
2. No real cloud immutable letter or pack read is wired to Core yet. A BLAKE3 hash of a letter does not sign its sender or prove a provider CAS. A malicious server could still present two self-consistent roots.
3. A single letter may only bridge one actual successor generation. A client that missed 2/5/20/100 generations must use the existing **[PR #197 per-generation history retention verifier](https://github.com/Lalalalendia/Akula/pull/197)** and then verify each generation's delta envelope or refuse fast replay. Do not collapse intermediate tombstones and sibling heads into a final diff.
4. The actual `AKPKEXP1` framing is still isolated in the tested pack prototype; complete per-member BLAKE3/J​SON/blob/DAG checks are required in the next read-only bridge before any letter can be used. Future E2EE needs authenticated encryption and key binding, not just unkeyed BLAKE3.
5. No S3 v2 writer, SDK-side CAS attempt, change to v1 key whitelist, background syncing, user file mutation, deletion, garbage collection, confirmed baseline or file gate authority in this PR.
6. Full current availability of previously pinned blobs cannot be inferred from the metadata cache; offline hints are insufficient for safe native apply.

## Required acceptance

- Default Rust Application checks and `sync-s3` feature compile/tests on the *exact final PR head*.
- Seven new negative/positive tests in `s3_coord_v2::tests` cover canonical schema, wrong vault, parent hash, historical siblings, tombstones, revision rewrites, missing members, broken pack tables, rollback and empty no-op successor.
- [MinIO CI workflow](https://github.com/Lalalalendia/Akula/blob/sync/coord-02a-v1-bound-envelope-contract-20261018/.github/workflows/sync-s3-minio.yml) explicitly runs `cargo test --locked -p aquilum-core --features sync-s3 s3_coord_v2::tests`. It does **not** claim to test a live S3 v2 protocol.
- No merge until all required triggered checks report successful outcomes. Preserve current V1 production and old 10k benchmarks unchanged.

## Independent next-stage tasks

**02b (read-only live MinIO):** read a bounded immutable v2 letter from S3, verify its exact V1 anchor using scope-bound SQLite journal and the standard manifest + per-generation history walker, then read bounded packs through the existing BLAKE3-checked SDK transport, verify each member JSON/blob and full DAG. Recheck mutable V1 root+ETag after I/O. The sidecar must never be used as an authority source instead of the canonical immutable V1 shard and full history.

**02c (only after 02b):** specify authenticated writer identity and root signatures/MAC, idempotent CAS publication, two-client history/merge and lost-ACK recovery, E2EE of letter+packs, V1/V2 capability negotiation and rollback/recovery. Preserve non-destructive File Gate acceptance as a separate production gate.

Sources: [Tela SYNC-COORD-01](https://telawiki.com/spaces/456/pages/7518/sync-coord-01-merkle), [Tela sync roadmap](https://telawiki.com/spaces/456/pages/7501/sync-delta-01-shard-reuse), [Notion SYNC-DELTA-01](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360).
