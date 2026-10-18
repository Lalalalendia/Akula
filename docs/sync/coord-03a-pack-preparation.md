# SYNC-COORD-03a — bounded V2 delta-pack preparation in Core

**Stage:** inert, production-compiled *pure bytes* encoder. No live sender, provider PUT, CAS publication, user-file modification, GC, background worker, baseline update, credential use or writer-authentication claim.

## Why this is necessary

The measured 02f 10k-note/0–7–1000-edit test from merged #238 uses a pack writer that existed only behind Rust `#[cfg(test)]`. Consequently, while the packed V2 *reader* has proven 16 GET for 7 updates and 316 GET for 1000 updates (on a warmed, old-anchor-backed, read-only 10k-note MinIO fixture), normal Akula could not produce that wire format. A test-only encoder cannot be treated as a usable sync implementation.

## What changed

- The already-existing, bounded `AKPKEXP1` byte builder in `pack_probe.rs` is now compiled with the opt-in `sync-s3` feature outside tests. It still has **no** S3 client or local-fs handle and is kept scoped to the internal S3 transport module.
- `coord_pack_encode::prepare_delta_packs_v2` accepts independently supplied immutable revision-ref, revision JSON and blob-byte triples. It sorts them by stable `RevisionId` and builds exact contiguous V2 pack index ranges. Ref/blob identity is validated by the same production decoder used in the real V2 MinIO reader before any prepared data is returned.
- Bounds match the existing V2 receiver: **1–1024 additions per batch; 1–128 additions per pack; ≤512 KiB per packed object; ≤16 MiB aggregate packed bytes**. A pack splits by member count **or by byte ceiling**. A zero-change batch is invalid: never synthesize an empty V2 letter. Duplicate UUID, incompatible ref identity, content-hash mismatch, tombstone/blob mismatch and excessive bytes fail closed.
- A packaged delta may legitimately reference a parent revision from the **previously verified old V1 snapshot**. The local encoder can preserve that unresolved DAG parent, but it cannot claim ancestry closure; only the reader joining the complete anchored old+new history may do so.
- The existing real 02f two-client MinIO fixture now uses this *production-compiled pure encoder* when preparing S3 packs and the V2 letter. Only the test fixture uploads them to the disposable bucket. New pure tests are included in the standard S3/MinIO CI gate.

## Acceptance and remaining authority barriers

Run exact-head Application/Core Rust, S3/MinIO, docs and 02f scale CI. Negative tests: 260 sorted exact members across 3 packs, 30 larger members split by **bytes**, old missing-parent acceptance as *not yet active* DAG, mismatched blob hash/document/UUID, zero and 1025 entries, member over 32 KiB and combined pack budget.

Before permitting **real** S3 uploads from the desktop client, implement and independently validate all of the following:
1. host-admitted guarded source bytes from registered physical vault and exact shared VaultId/profile, with a durable writer identity/capability;
2. immutable write-once upload/readback with index/letter publication under provider-enforced CAS and crash-durable lost-ACK recovery;
3. an authenticated discoverable per-generation delta locator/protocol negotiation — the present read-only 02e API still requires explicitly supplied letter digests and is NOT itself a self-discovering production feed;
4. safe complete File Gate apply, current old-blob availability guarantees, conflict handling and durable baseline advancement.

**Decision:** after this pure serializer passes CI, work on actual authorized publication and locator discovery rather than another synthetic coordinate tree or a premature opt-in "Sync now" action.
