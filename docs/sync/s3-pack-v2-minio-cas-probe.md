# SYNC-05d2h — Isolated MinIO V2 root CAS and lost-ACK recovery probe

**Status:** Test-only integration. The production V1 S3 transport, File Gate,
shared-vault profile, persisted remote anchor and automatic sync are unchanged.
The branch contains no V2 publication method callable by production code.

## Motivation

[SYNC-05d2g PR #208](https://github.com/Lalalalendia/Akula/pull/208)
added pure `S3PackedHeadV2`, `S3PackIndexRootV2`, strictly sized index shards,
immutable pack-member descriptors and complete-reference equality checks.
Those unit tests do not prove S3 server-side CAS, root validator stability,
byte-level readback or recovery when the network ACK goes missing.

[SYNC-COORD-02b PR #213](https://github.com/Lalalalendia/Akula/pull/213)
is an **independent** read-only compact delta sidecar on canonical pinned V1.
This probe deliberately does **not** compete with that V1-pinned design:
it exercises only a fresh disposable test vault whose current root uses V2
schema. There is no V1-to-V2 production migration.

## Proof sequence

The ignored test `minio_v2_pack_root_cas_two_clients_and_lost_ack`, invoked
explicitly in the existing S3/MinIO CI, performs:

1. Assert the endpoint is loopback HTTP MinIO. Create a fresh `VaultId`,
   construct two independently initialized official AWS SDK clients and a
   disposable per-test bucket. Never contact AWS regional S3.
2. Generate a base Markdown revision, two competing text edits and a
   tombstone. Upload every proposed immutable pack, V1 manifest/shard, V2
   index/shard with S3 `If-None-Match:*`; read back bytes and BLAKE3.
3. Persist a prepared V2 CAS intent in disposable SQLite
   (`WAL`, `synchronous=FULL`) before publishing an initial V2 root using
   the S3 server's real `If-None-Match:*` condition.
4. Verify from the second client using exact `HEAD → GET If-Match → HEAD`
   per immutable object and on the mutable root **both before and after**
   the complete verification. Decode the V2 root, read a digest-pinned
   manifest and all V1 revision refs, read a digest-pinned V2 index/shards,
   prove complete unique revision membership, fetch each content-addressed
   pack, check frame offsets and every JSON/blob BLAKE3, and close the DAG.
   Reject legacy V1 decoding of schema 2.
5. Both clients observe the same initial CAS validator. Each independently
   uploads its own, different immutable generation-2 graph, rooted in the
   exact generation-1 predecessor. The winner's root CAS is server-guarded
   with `If-Match`. Simulate a lost success ACK by discarding the receipt.
6. Re-open the persisted intent from a new SQLite connection; **read and
   authenticate the full current remote snapshot again** before recording
   an observed S3 ETag. Only byte-for-byte equality with the persisted
   candidate under the same vault/key permits an observed test receipt.
   This does **not** mark an Akula confirmed baseline.
7. The losing client's stale `If-Match` must receive HTTP 412 and its
   persisted intent must remain unresolved after a read-only comparison.
   The loser pack survives immutably; no automatic retry, merge or deletion.
   The winning branch retains both the text and tombstone sibling heads.
8. The other vault must reject the same root. Overwrite one test-only
   immutable pack with equal-length garbage, preserving the mutable V2
   root; subsequent full reads and receipt checks must fail closed.

## Important limitations

- **V2 live code is `#[cfg(all(test, feature="sync-s3"))]` only.**
  Only the canonical V2 **format contract** is present in production Core.
- This probe accepts a remote V2 current pointer within an isolated
  locally created vault. No production V1→V2 capability negotiation,
  migration, preserved signer identity, or secure writer authentication
  has been implemented. BLAKE3 proves byte consistency with a trusted root,
  **not** a user signature or authorization grant.
- The SQLite fixture journal is intentionally *not* the production
  SyncJournal schema. It proves prepared intent durability and read-only
  reconciliation, not crash-safe user file publication or sync baseline
  transition.
- This is a small DAG proof, not the 10k full packed cold benchmark and not
  a WAN/AWS S3 claim. The proof does not permit GC of abandoned packs,
  deletion of user notes, or using raw S3 listing as an authoritative
  snapshot.
- The reader authenticates the direct immutable V1 predecessor and all
  revision references of the current generation. Arbitrarily deep V2
  ancestry, offline devices, foreign-cloud status, 32KiB+ blob fallback,
  E2EE/key rotation, pack retention and TLS/provider matrices require
  separate work before runtime activation.

## Acceptance and next stage

Require exact-head S3/MinIO, Application and OS/file-durability gates green.
A test failure must block merging, not weaken hash/ETag checks.

On passing, a future 05d2i can implement a *non-default*, explicitly
admitted V2 **read-only** provider verifier, using the same size/hash/frame
caps and pinned vault binding. Writer activation requires later credential
authority, real durable journal, multi-device V1→V2 migration, E2EE and
deletion-retention review.
