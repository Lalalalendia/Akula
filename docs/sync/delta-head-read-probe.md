# SYNC-DELTA-01b — bounded, read-only S3 head observation

Status: experimental adapter probe. This document describes **implemented** read behavior in [PR #148](https://github.com/Lalalalendia/Akula/pull/148), not a working two-way synchronizer or a published delta wire protocol.

## Motivation

Naively comparing 10,000 remote notes through one S3 HEAD/LIST per object incurs substantial object/request latency. Akula already models a single vault-scoped immutable manifest plus a tiny `refs/current.json` pointer. Reading this pointer first can make *remote change discovery* independent of file count.

This does **not** imply O(1) work for local dirty-file discovery, snapshot construction, or the first sync, and it does not solve bulk small-file upload by itself.

## Two separated read-only operations

1. `observe_s3_current_head(client, keyspace)`: bounded GET of the tiny S3 head, checked by the existing transport's HEAD → conditional GET → HEAD, then strict `S3HeadPointerV1` decoding for the current vault.
2. A **caller that already possesses a verified, provider-profile-bound prior checkpoint** may compare the observed `generation + manifest ID + manifest BLAKE3 + ETag`. If unchanged, it knows only that the observed remote root matches that checkpoint. It **must still** process local pending operations and independently verify local state.
3. When the root differs, `read_s3_digest_matched_manifest(...)` downloads the referenced immutable manifest through a bounded GET, recomputes and checks BLAKE3, then independently rereads the root. Changed pointer bytes or ETag cause fail-closed `HeadChangedDuringRead`; retry requires re-observation/replanning.

Both functions are behind `sync-s3`, and both are read-only. The probe performs no S3 listing, no local file mutation, no CAS write and no SQLite baseline advancement.

## Important distinctions

- `S3HeadPointerV1::manifest_blake3` checks **byte integrity** against the root value, not writer authenticity. The new module does **not** parse or validate an immutable manifest schema or its causal ancestry; these belong to later SYNC-05d / SYNC-DELTA slices.
- The ETag of `refs/current.json` is an **opaque provider version guard**, not BLAKE3/MD5 of a content blob.
- An S3 profile/account/bucket/root mismatch invalidates persisted checkpoints even if a vault ID accidentally matches. A durable, profile-bound confirmed-head type has **not** been implemented; callers must not reuse these observations as such.
- The probe enforces 4 KiB for head, 1 MiB for manifest via the existing transport, plus object length and returned digest checks. Oversized data is an explicit error, not silently skipped.
- `NotFound`, permission denied, incomplete/changed reads, and checksum mismatches are errors, **never** evidence of deletion or of an empty vault.
- A verified head+manifest byte pair is an historical consistent read. Another writer may publish a newer head immediately afterward. Any future writer needs **fresh provider CAS** and the full conflict planner.
- This operation has 3 bounded object reads at the application layer (head, manifest, head) when the manifest is fetched. Current `get_bounded` itself uses HEAD/conditional GET/HEAD for each object: up to 9 HTTP requests; optimizing that overhead requires separate transport work with the same stability guarantees.

## Acceptance tests

- Read-only unchanged head: one application-level bounded read, no manifest download.
- New head: manifest digest comparison and strict recheck of current root.
- Concurrent CAS changes, identical pointer bytes with changed opaque ETag: fail closed.
- Wrong vault ID, corrupt head, missing head/manifest, oversized or malformed response: fail closed.
- No transition from these observations to file delete/write or baseline progression.

## Next dependency

Canonical, versioned immutable manifest schema and bounded delta chain verification; connect to `plan_revision_exchange` (merged [#143](https://github.com/Lalalalendia/Akula/pull/143)) only after cryptographic byte verification **and** causal/structural manifest validation, with profile-bound SQLite checkpoint support. Then measure 10k/100k-file cold/no-change/7-change scenarios against S3/MinIO. Packfiles remain a separately justified optimization.

Related Notion: [SYNC-DELTA-01](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360), [SYNC-PERF-01](https://app.notion.com/p/3fc32a84beec817a8d03de6745d64edd).
