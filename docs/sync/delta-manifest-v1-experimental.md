# SYNC-DELTA-01c — experimental immutable delta-manifest v1

**Status:** implementation experiment only. This document does not amend the canonical wire protocol in `protocol.rs`, enable cloud publication or mark stage SYNC-05d done. Source: [SYNC-DELTA-01 Notion](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360). Prerequisites: merged [#143](https://github.com/Lalalalendia/Akula/pull/143) (HAVE/WANT planner) and [#148](https://github.com/Lalalalendia/Akula/pull/148) (read-only S3 head/manifest probe).

## Why this separate format

For 10,000 existing immutable revisions and 7 new revisions, it is unnecessary to list/check 10,000 remote objects merely to discover which revisions appeared. The existing vault-scoped `refs/current.json` can point to the latest immutable manifest, while that manifest links cryptographically to an immediately preceding immutable manifest.

**S3 is passive:** clients never send a negotiation RPC to S3. They read the head and follow immutable objects. Clients may be offline for months; wall-clock timestamps and mtime must not determine causal order.

## Version 1 envelope

An experimental JSON `DeltaManifestV1` contains:

- Strict `schema = 1` and matching `vault_id`.
- `manifest_id` matching the immutable object key / head reference and `generation` matching the S3 head and predecessor chain.
- Optional `previous = {manifest_id, manifest_blake3}` (required after generation 1; forbidden at genesis).
- Canonical ascending list of new immutable `revision_ids` (no plaintext file paths or filenames).

The head remains unchanged: `S3HeadPointerV1` has `manifest_id`, `manifest_blake3`, `previous_manifest_id` and `generation`. The payload repeats predecessor ID and adds predecessor BLAKE3 to make the full chain of historical manifests checkable. The existing `S3Keyspace` naming is unchanged; `protocol.rs` has no new required feature negotiated or remotely published.

**Limitations:** at most 1 MiB per manifest, 4,096 new revision IDs per manifest, 128 generations traversed by default, 100,000 accumulated IDs. Exceeding the caller's bounded chain budget asks for a **separately verified full snapshot**. There is no implementation of full snapshots or retention yet.

## Inspection semantics

`inspect_delta_manifest_chain(keyspace, head, checkpoint, newest_first_bytes, limits)` is a pure, read-only validator:

1. Validate root/checkpoint against keyspace; reject rollback or same-generation forks.
2. Match the actual BLAKE3 of raw immutable manifest bytes to the expected digest from the head or child manifest.
3. Decode strictly: reject unknown authority fields, unsupported schema, foreign vaults, invalid generations/previous links and non-canonical/duplicated revision IDs.
4. Check current head's previous ID matches the latest manifest's previous ID; advance one generation at a time.
5. If the validated parent identity/hash matches the previously confirmed checkpoint, return only the revision IDs from the intervening manifests.
6. If no checkpoint exists, a complete genesis traversal may be returned as **bootstrap metadata only**; the first-sync preview/consent gate still applies.
7. When not enough older frames were supplied, return `NeedOlderManifest` with exact immutable object ID, expected hash and generation. Re-read this ancestor from S3 and rerun validation. A long/foreign/divergent history instead gives `RequiresVerifiedSnapshot` (never a partial safe plan).

**Integrity != authenticity.** Relative BLAKE3 links alone do not authenticate the publisher. The caller MUST validate remote identity and authorization, bind checkpoints durably to provider profile/root, and enforce anti-rollback policy separately. E2EE (SYNC-06) must authenticate or encrypt manifest metadata and revision payloads before activation.

**Metadata != payload availability.** The returned IDs still require verified immutable revision records, parent closure via `RevisionDag` and guarded HAVE/WANT/planner. Physical files remain protected behind `File Gate`. A tombstone cannot delete anything in this module.

## Benchmarks and acceptance

- Remote unchanged: only `refs/current.json` needs to be observed; local pending changes must still be checked.
- Remote +7 of 10k: traverse one immutable delta manifest after a verified prior checkpoint; ensure seven IDs are returned and S3 object count is constant with respect to unchanged files.
- Offline two/three generations: verify links newest-first, return revisions in chronological generation order; no timestamps.
- Errors: duplicate IDs, altered bytes, corrupted predecessor, forked/rolled-back head, missing ancestor, profile/vault mismatch, malformed/oversized data and future schema.
- Race: even after a successful read, a different writer can publish a new head; future publication must use provider-enforced CAS, not blind overwrite.
- No writes/network/listings in this implementation. No performance improvement is claimed until actual MinIO and remote S3 A/B measurements.

## Next PR

Read-only S3 orchestrator: bounded fetch of missing ancestor by `NeedOlderManifest`, integrity check through the SDK, refresh latest head, then run the pure validator and existing `RevisionDag` / `plan_revision_exchange` with a confirmed snapshot. Add verified immutable-snapshot fallback and a profile-bound durable checkpoint before any publication. Do not enable actual two-way sync until SYNC-04 safety gates and SYNC-05c/d/e are complete.
