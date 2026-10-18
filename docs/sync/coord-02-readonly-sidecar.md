# SYNC-COORD-02b — read-only S3/MinIO verification of bounded V2 letters and immutable packs

**Scope:** optional V1-anchored *read-only* integration. This is intentionally **not** a new production S3 protocol writer, not an automatic sync mode, and not native file application. It extends the [merged SYNC-COORD-02a v2 metadata contract](https://github.com/Lalalalendia/Akula/pull/206), while the strict V1 root, data model, CAS history, complete DAG and full blob-availability verifier remain authoritative.

## Why

The full [10k-notes A/B](https://github.com/Lalalalendia/Akula/actions/runs/39081242570) showed the physical value of a compact changed-revision letter and bounded immutable transaction packs: 7 edits required 2 GET in the experimental fixture, 1000 edits required 9 GET. The fixture *assumed* a known old root; it did not independently pin real V1 state, authenticate writers, test a production V2 reader or authorize File Gate writes.

This slice integrates the existing strict letter and pack verifier with the actual official AWS SDK and a **real disposable localhost MinIO** pair of clients, while preserving the existing V1 assurance level. Any claim that this slice itself performs whole-vault sync in two GET would be false: it runs expensive full V1 pinning first.

## New API

`AwsS3ProbeClient::inspect_v2_sidecar_against_pinned_v1(scope, anchor_journal, old_pinned_snapshot, old_verified_metadata, expected_letter_digest)`

Returns **only** `S3CoordSidecarPreview` (generation, newly checked revision/pack counts, verified wire pack bytes and IDs, explicit `requires_full_apply_gate=true`).

### Read sequence

1. **Before network access:** compare actual configured S3 SDK bucket/region/endpoint/path-style/prefix/vault with the provided `S3RemoteAnchorScope`. Fail closed for foreign scope, missing bound SQLite anchor, stale/wrong old pinned snapshot ID or stale/full-DAG-incomplete prior metadata cache.
2. Independently revalidate the old immutable V1 manifest hash against the durable SQLite S3 anchor, decode the strict V1 schema.
3. Independently run canonical `read_verified_head` **and a full `pin_revision_snapshot` for the current V1 root**. These methods read and validate the current complete V1 remote history and blob availability. This avoids accidentally creating a weaker new source of truth from an untrusted letter, but it is slower; it is a correctness integration stage, not a production request-saving benchmark.
4. Read one caller-referenced immutable **content-addressed** sidecar with bounded HEAD / If-Match GET / HEAD (hard cap 512KiB); verify exact BLAKE3, strict/canonical `S3DeltaEnvelopeV2`, both old/current V1 pointer and manifest hashes, one direct successor, and 100% revision reference retention and changed-ID coverage against the two independently pinned V1 snapshots.
5. Seed a disposable `RevisionDag` from the fully checked old revision JSON metadata. Read each bounded pack as immutable S3 `blobs/<BLAKE3>` with full HEAD / If-Match GET / HEAD, check declared length and digest, verify the key and every member JSON hash, typed revision and document IDs, blob length/hash and tombstone operation with the existing `pack_probe::ingest_verified_pack`; demand one complete DAG with **zero pending parents**. The pack builder remains strictly `#[cfg(test)]`; no V2 pack publisher is activated.
6. Require the observed new UUIDs to exactly match the entire letter, and the DAG size to equal the fully pinned V1 current revision count. Finally **re-read** V1 `refs/current.json` and compare both pointer value and S3 ETag to the independent initial verified head. A racing CAS must cause Hold, never a user-file change.
7. Return read-only advisory counts, not `RemoteStore`, `StagedDownload`, `ConfirmedBaseline` or File Gate write receipt.

### Physical compatibility

For this integration the disposable MinIO test deliberately uploads **both** the legacy V1 individual `revisions/<UUID>.json` and `blobs/<hash>` objects **and** the V2 compact pack+letter sidecar. This is **dual-storage write amplification**, not yet a completed native V2 migration. Old clients keep reading V1 normally and must never encounter a new field or version in V1 `refs/current.json`. A future V2-only writer must provide capability negotiation, versioned root and safe migration, tested across old and new clients, before removing legacy objects.

## Tests and attack simulation

- Real separate MinIO sender A and consumer B, fresh randomly named disposable bucket and VaultId, loopback endpoint exclusively; consumer independently full pins genesis, writes the protected private SQLite remote anchor, then A atomically advances V1 generation 2 with **two different sibling revisions** sharing the same parent: one edit and one tombstone. Sidecar with sorted new refs and one pack reads successfully on consumer B and returns only advisory receipt.
- **Missing-pack attack:** publish a self-consistent sidecar with a valid expected letter hash and a pack digest whose S3 object does not exist. Must return an explicit error, never empty remote or deletion authority.
- **Rehashed-corruption attack:** mutate a pack member, then recompute the outer pack and letter hashes and upload the altered pack. The independent V1 revision/blob hashes reject the payload despite apparently valid envelope digests.
- **Stale-generation attack:** V1 is atomically advanced again to generation 3. The old one-generation letter cannot be used to skip generation 2; it must be rejected by V1 ancestry gates. Durable previous SQLite anchor never advances from exploratory sidecar reading.
- Existing pure strict tests verify foreign vault/root, noncanonical schema, UUID rewrite/missing sibling or tombstone, pack range holes/repeats and missing history. All old MinIO/Windows/macOS/Linux File Gate checks remain in effect.

## Interaction with the separate inert V2 packed-index proposal

The repository also contains an **inert** S3PackedHeadV2 / S3PackIndexRootV2 / S3PackDescriptorV2 schema in [s3_pack_index.rs](https://github.com/Lalalalendia/Akula/blob/main/aquilum-app/core/src/sync/s3_pack_index.rs). That is a possible **whole-snapshot packed index**, distinct from this **per-generation compact change letter**. Neither is an active root or writer. This PR does not introduce an alternative mutable V2 root, enable two independent authoritative indexes or alter V1 keyspace. A later protocol integration must unify these components under **one** authenticated, capability-negotiated CAS root and a single retention/migration design, rather than maintain competing user-visible sync engines.

## Known limitations, admission rules

- **No request reduction claimed.** This bridge runs V1 full pinning first; use an independent later benchmark after a proven selective pack reader can replace new JSON/blob GETs *without* weakening old head, CAS, per-generation history or current availability proofs.
- Unkeyed BLAKE3 does not authenticate a writer, grant anti-rollback provenance against hostile local DB replacement or encrypt path/document metadata. An attacker controlling an entire S3 writer credential/profile can still replace self-consistent but unsigned heads; remote writer authentication and E2EE remain open.
- One letter covers only a direct V1 successor. After extended offline, use existing [PR #197 retention](https://github.com/Lalalalendia/Akula/pull/197) across all missed generations, then check each letter or Hold. Never silently accept only newest.
- Previous immutable blob availability must be freshly established before any local File Gate application. This read-only verification does not grant local user writes, deletes, garbage collection, CAS publish or baseline advancement.
- No S3 PUT is added to normal application code. All PUTs in tests occur **only** in disposable localhost MinIO with no user vault paths.

## Follow-up

**02c:** move the observed security proof to an optimized incremental route: profile-bound verified prior anchor, strict V1 intermediate history and current immutable shard references, bounded letter+pack reads, complete DAG and head/ETag recheck, *without* a redundant full pin. Measure actual end-to-end S3 GET/HEAD under 0/7/1000 edits and realistic WAN RTT.

**02d:** authenticated writer/capability negotiation and crash-safe CAS publisher with E2EE and V1/V2 migration plus read-only two-device conflict/restart tests. Native File Gate authorization remains a further separate gate.

References: [SYNC-COORD-01 Tela](https://telawiki.com/spaces/456/pages/7518/sync-coord-01-merkle), [Notion project](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360), [V1-bound v2 letter contract](https://github.com/Lalalalendia/Akula/blob/main/docs/sync/coord-02-v1-bound-envelope.md).
