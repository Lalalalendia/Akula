# SYNC-05d2g — inert V2 pack-index contract and guarded-root design

**Status: descriptive contract only.** Source:
`aquilum-app/core/src/sync/s3_pack_index.rs`. It introduces pure,
strictly bounded wire codecs and a complete-membership validator. **There is
no V2 S3 reader/writer, CAS publisher, migration, scheduler, File Gate write,
cloud delete/GC authority, receipt journal or confirmed-baseline advancement.**
Importing these types cannot enable sync.

## Why

The merged [10k full-DAG benchmark #183](https://github.com/Lalalalendia/Akula/pull/183)
required **20,240 GET + 40,480 HEAD** for 10k revision JSON and 10k blobs.
The merged [pack A/B #194](https://github.com/Lalalalendia/Akula/pull/194)
showed that 79 disposable immutable packs could carry the same data with
**79 GET + 158 HEAD and 660ms** of member reads on localhost MinIO, but its
pack index was created in test RAM from an **already authenticated V1 root**.
That 660ms is **not** security-equivalent production sync time.

This stage closes the **schema and identity-binding** part of that gap. It
does **not** close S3 transport, key management, migration or crash recovery.

## Proposed trust chain

A **future** V2 client, after a provider-enforced conditional read of its
vault's exact `refs/current.json`, would validate:

```text
S3PackedHeadV2 (schema=2, vault, generation, manifest id/hash,
                pack index id/hash, previous root BLAKE3)
   | fixed current-root object + provider CAS / persisted receipt
   v
S3PackIndexRootV2 (same vault/generation + identical V1 manifest id/hash)
   | BLAKE3 over bounded, canonical JSON
   v
S3PackIndexShardV2 × 40 for ~10k small revisions (<=128 KiB each)
   | BLAKE3 over each shard
   v
S3PackDescriptorV2 (<=512 KiB per pack; <=128 members)
   | content-addressed hash + size + member frame offsets
   v
AKPKEXP1 packet and each exact S3RevisionRefV1 + verified JSON/blob BLAKE3
   | revision parents and tombstones verified before local use
   v
Complete verified DAG, never a deletion authority by itself
```

`S3PackIndexRootV2::verify_complete` checks **exact** identity of every
member in all shards against an independently authenticated list of
`S3RevisionRefV1` from the pinned `S3ManifestV1`: revision ID, document
ID, revision BLAKE3 and content size. Missing, duplicate or foreign members
fail closed. No raw `ListObjectsV2` result can establish this snapshot.

A parsed or caller-created `S3PackedHeadV2` by itself is **not** a trusted
root. Only future S3 provider root/CAS verification and durable journal
reconciliation can establish root authority. Rehashing a malicious index
is useless only when its parent root checksum is independently trusted.
The existing V1 decoder rejects schema 2 and must continue doing so.

## Hard limits

| Object | Upper bound |
|---|---:|
| V2 root pointer | 4 KiB |
| Index root | 128 KiB |
| Index shard | 128 KiB |
| Number of index shards | 256 |
| Packs per index shard | 2 |
| Entries per packed S3 object | 128 |
| Packed object | 512 KiB |
| One revision JSON | 64 KiB |
| One packed content blob | 32 KiB |
| Total revisions in index | 32,768 |

Pack frames use the test-validated `AKPKEXP1` framing:
8-byte magic, little-endian 16-bit count, then each member's
32-bit JSON length, 32-bit blob length, exact JSON and blob bytes.
`revision_offset` and `blob_offset` are computed from each contiguous
frame and must not overlap or point beyond the pack. Tombstones have a
revision JSON and **no** blob offset or blob length. Content or rename
revisions require a separately verified blob BLAKE3 during content read.

Objects larger than 32 KiB do **not** silently fit the pack format.
Fallback to individual immutable blobs requires an explicit, later
versioned index representation; until then an incomplete index is Hold.

## Root transitions and unknown ACK

`S3PackedHeadV2::is_direct_successor_of` is a **pure predicate**:
same vault, generation increments by one, different immutable manifest ID,
and exact BLAKE3 link to the previous serialized V2 root. It does not
perform a CAS, prove remote existence, confirm a baseline or authorize a
blind retry after a lost ACK.

Upgrading an **existing** V1 vault to V2 is NOT implemented here:
the V2 genesis/ancestry rules cannot silently reinterpret a V1 root.
A future migration must create a durable prepared intent, authenticate
both roots, preserve all revisions and undeleted blobs, publish through
the actual provider `If-Match` validator, and independently verify which
generation won on unknown ACK. The old V1 reader must Hold rather than
mistakenly recognizing a packed V2 root.

## Test matrix

- V2 root/index/shard canonical roundtrips and old V1 decoder rejection.
- Foreign VaultId, unknown schema, unknown write/delete authority fields,
  oversized root/index/shard and invalid generation/ancestry.
- Corrupted index shard, modified member reference after rehash, missing
  shard/revision, duplicate ID, forged offsets, oversized pack and unknown
  member bytes.
- Exact preservation of offline sibling heads and tombstone references.
- Entire 10k small-revision index in 40 shards without exceeding bounds.
- Wrong previous root digest cannot satisfy direct-successor check.

These are contract tests only. **Still required before any V2 production
runtime:** disposable live MinIO V2 root CAS + lost-ACK double-writer
reconciliation; exact S3 HEAD/GET If-Match/HEAD reads for every index shard
and pack; independent complete-member DAG verification; full 10k cold A/B
including downloading/authenticating the new root/index; SQLite crash
recovery; E2EE and key rotation; large-blob fallback; orphan pack/MPU
retention; real AWS/WAN provider tests and all native File Gate gates.

The V1 sync pipeline is intentionally untouched.
