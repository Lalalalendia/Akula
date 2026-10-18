# SYNC-05d2i — Explicitly pinned, profile-bound V2 read-only inspection

**Status:** opt-in library method only. This is neither the Akula sync
scheduler nor a `RemoteStore` implementation. No cloud publication, local
File Gate write, baseline advancement, listing-as-snapshot or garbage
collection is enabled.

## Security admission — required before the first S3 request

`AwsS3ProbeClient::inspect_explicitly_pinned_v2_read_only(...)`
requires four independent inputs:

1. A caller-created `S3RemoteAnchorScope` with the exact bucket, region,
   endpoint, path-style setting, vault namespace, profile ID and **trusted
   account binding** (the same scope type already used by V1 pinned anchors).
   The actual SDK client's connection must exactly match it **before network
   requests**. A different VaultId or profile is a hard refusal.
2. The **complete expected V2 head** from a separately trusted source. The
   reader compares its canonical bytes byte-for-byte to the mutable remote
   `refs/current.json` returned by S3 HEAD/GET If-Match/HEAD.
3. The expected opaque **current-root ETag**, also from the independently
   trusted checkpoint. This is a CAS validator, NOT a BLAKE3 or an account
   signature. The root is read twice (before and after all content) with
   exact byte+ETag equality.
4. For generation >1, the independently trusted **immediate predecessor V2
   head**. `is_direct_successor_of` must pass; current immutable V1 manifest
   must name the same predecessor's pinned manifest ID/hash, and all its
   revision references must remain present unchanged. Without the previous
   head, the verifier **fails closed**. Generation 1 rejects an unexpected
   predecessor.

There is **no V2 head-on-first-use auto-trust** and no V2 SQLite binding
migration here. Passing the hash/ETag of whatever the same untrusted server
just returned back as a "trusted" input is NOT acceptable onboarding and
does not prove the writer. Persistent V2 anchor creation, identity custody,
anti-rollback protection and capability negotiation need separate explicit
user-authorized machinery.

## Full integrity proof after admission

The method downloads all content in strictly capped S3 reads using the
existing official AWS Rust SDK (HEAD → GET If-Match → HEAD) and validates:

- V2 schema and VaultId of the pinned root, exact generation, index and
  immutable V1 manifest IDs/BLAKE3. Unrecognized V2/V1 content is rejected.
- Immutable manifest and all revision-reference shards, BLAKE3 and shard
  boundary identities (no S3 LIST used), no revision duplicates or missing
  members; direct parent manifest and full unchanged-reference subset.
- Immutable V2 index and all index shards with their own BLAKE3 digests;
  exact full member coverage through existing
  `S3PackIndexRootV2::verify_complete`.
- Every packed blob (≤512 KiB), exact `AKPKEXP1` frames and member
  offsets, BLAKE3 of each revision JSON and every contained content blob.
  Tombstones have no content, all conflicting sibling edits remain in a
  complete, parent-closed `RevisionDag`.
- The mutable root after all immutable reads: any changed bytes or ETag
  invalidates the entire preview. Missing, stale, oversized or corrupt
  remote objects are **errors**, never evidence for local deletions.

The only returned value is `S3PinnedV2ReadPreview`: number of revisions,
packs, downloaded bytes, current generation, count of logical DAG heads
and a mandatory `requires_explicit_file_gate_review=true` marker.
No file bytes, handles, writer methods, sync receipts or credentials escape.

## Tests and boundaries

`s3_transport/v2_readonly_tests.rs` covers independent member hashes,
even when an attacker re-hashes the outer pack; duplicate revisions;
conflicting siblings and tombstones; wrong blob offsets and trailing bytes.

The existing disposable two-client
`minio_v2_pack_root_cas_two_clients_and_lost_ack` now calls this real
production-compiled verifier at generation 1 and after a winning generation
2 CAS. It checks wrong current ETag, stale root, missing predecessor, and
a **foreign-vault scope rejection without issuing a network request**.
Same-length corruption of the test-only pack is rejected by the new
verifier as well as the old test-local reader.

**Distinct workstream:** [PR #213](https://github.com/Lalalalendia/Akula/pull/213)
offers a V1-pinned read-only compact delta sidecar. This PR adds the
**independent V2-root read path**; it does not invent a second mutable
production root or silently activate migration.

**Not implemented:** V2 persisted trusted initial anchor, trusted signer
authentication / E2EE, arbitrary deep immutable V2 head history, large
blob fallback (>32 KiB), bounded network concurrency for a full 10k index,
journalized V2 publication, File Gate staging, two-device release migration,
remote delete/GC or AWS/WAN acceptance. All remain Hold conditions before
user-facing automatic synchronization.
