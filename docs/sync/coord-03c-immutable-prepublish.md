# SYNC-COORD-03c — S3 immutable sidecar prepublication before root CAS

**Stage:** production-compiled, **crate-internal** S3 writer for *immutable* V2 pack/letter/locator bytes; no UI entrypoint, native File Gate operation, protected original mutation, profile credential persistence, garbage collection or confirmed sync baseline. Follows merged #243 (pure byte encoder) and #248 (deterministic anchored V2 letter discovery).

## Why

The live two-client test for #248 demonstrated that putting a new V1 `refs/current.json` root **before** the corresponding immutable locator creates an observable period where the canonical root has advanced but the fast V2 discovery must HOLD. The existing V1 root CAS already has a durable SQLite WAL; it should not be replaced by another ad hoc S3 PUT.

## Implemented invariant

New internal SDK helper `prepublish_coord_v2_immutables` accepts a profile/vault-bound remote scope, an independently verified previous V1 root+ETag and its complete pinned revision snapshot, a proposed one-generation successor V1 head+manifest already uploaded immutably to S3, a strict V2 delta letter, and packs produced by the production `prepare_delta_packs_v2` byte encoder. Before *any* V2 PUT:

1. Reject foreign scope, wrong predecessor, changed generation, reused/split identifier, wrong canonical manifest hash, invalid pack/letter range or mismatched V2 refs.
2. Verify that the **current** remotely observed V1 head AND ETag still equal the independently verified old root.
3. Fully pin the **candidate immutable V1 manifest**, including all old/new revision JSON and current blob availability. This conservative first writer check can be expensive at 10k notes; it is a correctness boundary, NOT an optimization.
4. Validate the exact letter membership against both fully pinned V1 states, preserving all previous sibling and tombstone revisions; every new V2 pack index/offset/digest must match the already self-verified packed bytes.
5. Upload immutable **packs → letter → deterministic locator** in that order, under provider-enforced `If-None-Match:*`. A precondition failure or lost ACK is settled only through bounded HEAD → conditional GET → HEAD and exact BLAKE3 **and byte** readback, never blind overwrite.
6. Recheck the unchanged remote V1 root+ETag. If another writer won during staging, return refusal; any staged, unreferenced objects remain immutable and are **not deleted**.

The result is an *internal descriptive preview only*. It cannot call `publish_head_cas` for the caller. A separately authorized caller must still persist the exact intended S3 root/old ETag in `S3HeadJournal` and use its existing provider-enforced CAS/recovery protocol. Local writer identity and source custody are NOT derived from a scoped S3 string or from BLAKE3.

## Real MinIO acceptance

Extend the preexisting `live_minio_v2_two_offline_generations_retains_tombstone_and_rejects_gap` two-client fixture, already executed in the standard S3/MinIO `live_minio_` CI lane. Keep gen2 **root-before-locator** as a negative proof that missing locators HOLD; for gen3, publish original immutable V1 revision/tombstone and manifest, then prepublish V2 pack+letter+locator **before** changing root:

- Corrupt pack bytes must be refused before any of the three V2 objects are uploaded.
- Repeating exact immutable prepublication is idempotent even when `IfAbsent` rejects duplicate keys. The remote V1 root remains generation 2 until the separate CAS.
- The other client must still observe only generation 2 before CAS, despite complete staged generation-3 sidecars.
- Use the actual `S3HeadJournal` plus `publish_head_cas` to atomically publish generation 3. Repeating the SAME operation ID must recover as the same verified root without blind replay.
- The receiving client discovers both letters using only its saved generation-1 SQLite anchor and the server's canonical V1 immutable ancestry, retaining both sibling and tombstone. Existing gen4 omission of a past tombstone must remain HOLD.
- All old 02f 10,000-note read-only modes and required Rust/S3 platform checks must remain green on the tested PR head.

## Still blocking production synchronization

- **No host-authorized local sender**: neither a locally registered note nor its unsaved editor state can supply publish authority solely by handing this function raw revision bytes. A real host-owned sender must bind OS custody, documented operation intent, durable source bytes and independent remote account/bucket identity. PR #249 explores AWS STS/bucket-owner proof but is not by itself a writer capability; MinIO needs its own explicit trust boundary.
- **No automatic root CAS or File Gate apply**: generation-3 CAS in this PR occurs only in the isolated integration test through the *existing* SQLite journal. The production application still has no enabled remote push/pull loop or per-document confirmed baseline.
- **No present-day old-blob preservation claim after selective discovery**: the full candidate pin is a conservative prepublication gate; it does not turn read-only 02e discovery previews into local mutation rights or protect against later remote deletion/rollback.
- **No encryption/privacy claim**: plaintext notes inside V2 packs remain visible to an S3 provider unless separately encrypted under a future versioned capability negotiation.

Next actual product integration should bind the already implemented immutable stage and existing crash-durable CAS to a verified host source and account/custody authority, then pair it with an explicit File Gate apply decision. Do not replace this with another coordinate-tree benchmark.
