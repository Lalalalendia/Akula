# SYNC-05d2f — experimental immutable S3 packs (not a production protocol)

Status: **test-only experiment**. Do not register the format in normal Akula
sync, do not promote a baseline or infer rights to delete remote/local objects.

## Why this experiment exists

The [merged PR #183](https://github.com/Lalalalendia/Akula/pull/183)
verified **10,000 revision JSON objects and 10,000 content blobs** from a real
disposable localhost MinIO server. Its accepted final-head run measured
**20,240 GET + 40,480 HEAD** and **72.876 seconds** for fully verified S3
head readback, despite only **19,505,039 bytes** of checked payload.

A separate [PR #189](https://github.com/Lalalalendia/Akula/pull/189) evaluates
warm and persisted SQLite revision hints. These are **not substitutes**:
metadata caches avoid *repeat* reads; packing targets *first/cold* retrieval
when tens of thousands of small objects would otherwise require tens of
thousands of SDK operations.

## Experiment boundaries

The experimental `pack_probe.rs` is compiled **only under `#[cfg(test)]`**.
It has no public API, provider capabilities, persistent database, user
filesystem handle, credential persistence, implicit update or delete path.

Each pack is a small content-addressed object in a throwaway **test-only**
vault. It is written with S3 `If-None-Match: *` and read via the existing
`get_bounded` HEAD → GET(If-Match) → HEAD, with SDK BLAKE3 verification.
The production `S3Keyspace` validation, `S3ManifestV1` and `RemoteStore`
do **not** accept packed members.

| Bound or invariant | Experimental value |
|---|---|
| Envelope | 8-byte magic `AKPKEXP1`, LE u16 count |
| Member frame | LE u32 revision JSON length, LE u32 blob length, canonical JSON, blob bytes |
| Members per pack | 1–128 |
| Packed object size | ≤512 KiB |
| Revision JSON | ≤64 KiB |
| Blob within pack | ≤32 KiB (larger objects would need normal immutable blobs) |
| Concurrent pack S3 GETs | ≤8, each ≤512 KiB |
| Cached fixture memory | ≤24 MiB for this 10k experiment |
| Trust checks | BLAKE3 of envelope and each JSON/blob, exact revision/document/size identity, strict EOF, uniqueness, complete DAG parent closure |
| Tombstones and forks | No blob for a tombstone; its JSON and all competing heads must be preserved |
| Dangerous actions | No production publishing, overwrite, GC, baseline change or File Gate write |

**Important trust distinction:** The pack descriptor (S3 key, BLAKE3,
ordered `S3RevisionRefV1` entries) is generated in memory by this test. Its
entries are compared to the *already fully verified* current S3 manifest from
the original protocol. This proves byte-level equivalence and request
reduction, **not** that an untrusted client can discover/authenticate
packs without separately downloading the old objects. The timed pack phase
does not include a future versioned pack index or its root CAS.

## Paired read benchmark

The opt-in full-DAG CI still seeds the original 10,000 JSON + 10,000
independent 1-KiB blobs, 79 shards, and two CAS-root generations. At the same
time, it prepares 79 immutable packed sidecars containing identical members.

1. Read and independently verify the **original** S3 root, all 10k JSON,
   all 10k blobs, and DAG closure. Record `raw_verify_ms`, SDK GET/HEAD.
2. From the **same server and immutable revision set**, read at most 8 packs
   concurrently with exact digest and member verification, rebuild the DAG,
   require exact equality with every revision in the already checked manifest
   and preserve the sampled three-way offline fork. Record `packed_read_ms`,
   SDK GET/HEAD, bytes, and separately timed pack seeding.
3. Overwrite one test-only packed sidecar with same-length garbage, then
   require digest failure. Unit tests also reject tampered JSON, tampered blob,
   truncation, extra bytes, invalid references and oversized packs.

A valid positive result is fewer SDK calls on *cold packed data*: e.g. 79
GET and 158 HEAD rather than >20,000 GET and >40,000 HEAD. A lower elapsed
time would be **local MinIO evidence for this exact setup**, not a WAN/AWS
speedup estimate; upload/publishing a new pack index is not included.

The workflow remains opt-in via `sync-s3-full-dag-scale.yml` when the
benchmark code changes, and by explicit manual dispatch. Normal CI runs
lightweight pack framing/tamper tests.

## What must be resolved before any real pack protocol

- Introduce a new, strictly versioned, vault-bound manifest/index that pins
  pack digest, members, byte ranges and object identity **atomically** with
  the regular root CAS; preserve old readers as incompatible/Hold rather than
  silently changing their interpretation.
- Define E2EE, key rotation, maximum ciphertext/member framing, decompression
  limits, bounded range reads and fallback for large individual blobs.
- Prove write-once pack upload and lost-ACK reconciliation with journaled
  receipts. Abandoned packs must remain harmless; orphan/MPU cleanup requires
  an independent GC/lifecycle authority and offline-device retention rules.
- Run network experiments with realistic WAN RTT, 0/7/1000 changed revisions,
  partial pack reuse, stable vault binding and two concurrent offline editors.
  Reject any pack optimization that loses tombstones/sibling heads or uses
  S3 LIST as a stable snapshot.
- Re-run the File Gate, native cross-process durability and recovery tests
  before granting user-facing sync authority.

No S3 object is to be deleted on the basis of this experiment.
