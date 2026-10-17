# SYNC-05 — S3/MinIO provider: transport, immutable state and acceptance plan

**Status:** design and offline S3-05a contract; **no live S3 sync or local write authorization**.
**Canonical dependency:** [Tela PR 00–11](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11), stage **05 depends on accepted stage 04**.
**Implementation home:** `aquilum-app/core/src/sync/`, no separate Python service, no dynamic DLL or second planner.
**Date:** 2026-10-17.

## Decision and scope

- **First provider:** local MinIO for deterministic integration, then verify real AWS S3 behavior. Other S3-compatible endpoints are supported only after passing the same guarantees (not by brand label alone).
- **One active remote profile per vault in v1.** No simultaneous writable S3+Google+Yandex bridge in this stage.
- **Local data stay plain Markdown and attachments.** S3 stage 05 keeps immutable blob/revision history and a CAS-controlled manifest, not a filename-for-filename mirroring tool. A user-facing plaintext mirror and importing third-party edits are stage 10.
- **Not a cloud-sync service:** peers exchange through S3 using the existing `RemoteStore`, SQLite WAL journal, DAG planner and File Gate. Network execution must be in a Core worker, never in the native UI event handler.
- **Preservation first:** on conflicting edits/deletes, retain every known branch, show conflicts. Never delete local or remote history based solely on timestamps, absence from a listing, or a server error.
- **No credentials in vault, Git, ordinary JSON settings, crash logs or object keys.** Desktop credentials should come from an OS-protected store / constrained AWS credential provider; no embedded permanent access key.
- **Stage 05 is initially unencrypted:** a plaintext BLAKE3 object key exposes a content fingerprint and is NOT encrypted metadata. Stage 06 must use opaque protected object names and encrypt path metadata and content; enabling encryption on an existing profile is a separately tested migration.

## The provider-neutral contract is already present

Read `sync/provider.rs`, `journal.rs`, `upload.rs`, `download.rs`, `dag.rs` and `vault.rs` before enabling a provider. The current `RemoteStore` requires explicit conditional operations and `collect_verified_listing()` rejects providers with `snapshot_consistency=false`.

**Key distinction:** AWS S3 provides strong read/list consistency for individual API operations; `ListObjectsV2` does not give Akula a point-in-time, multi-page, application-revision snapshot identifier. Never invent `RemotePage.snapshot_id` using an S3 continuation token, clock, or a single listing request. A `ListObjectsV2` result can help discover abandoned objects for manual repair, **never** justify deletions or baseline advancement.

To satisfy the current `RemoteStore`, expose a **verified manifest-backed logical listing**, pinned by a validated immutable manifest and a conditional current-pointer protocol. This is an Akula-level snapshot, not a vendor listing snapshot. The adapter must fail closed if it cannot prove a consistent manifest. This stays independent of S3 server versioning and its ETag metadata.

## Provisional S3 v1 namespace (05a, offline contract)

The prefix is a simple ASCII slash-separated namespace controlled by Akula, e.g. `akula-sync/desktop`. User note paths are NEVER used as S3 object keys. Example:

```text
<configured-prefix>/v1/vaults/<vault-uuid>/
  refs/current.json                     # the ONE mutable pointer, provider CAS
  manifests/<operation-uuid>.json        # immutable graph-head snapshot
  revisions/<revision-uuid>.json        # immutable per-document DAG revision
  blobs/<blake3-hex>                    # immutable plaintext blob in stage 05
```

The Rust `S3Keyspace` refuses unsafe/ambiguous prefixes. `S3HeadPointerV1` is a bounded (4 KiB), strict JSON envelope containing a schema revision, vault binding, monotonic generation, manifest object ID, BLAKE3 of exact manifest bytes, and previous manifest ID. Every read must verify vault/schema/hash and reject unknown authority fields; unknown schema -> fail closed. A pointer does not by itself prove reachability: later stages verify actual immutable manifest bytes and their referenced DAG.

**Important boundary:** `S3HeadPointerV1` validates an individual pointer and its direct predecessor, not a global remote transaction. Stage 05d must implement multi-generation ancestor traversal, live CAS, readback and recovery before declaring safe synchronization.

## Remote commit protocol (05c–05d)

1. Acquire local profile lock, read current pointer with its opaque S3 ETag, fetch/verify immutable manifest bytes and dependency references, compare against locally confirmed baseline.
2. Collect local snapshot through `CoreVaultScanner` and the only authorized `VaultPort`/File Gate; treat dirty editors and access/alias failures as explicit holds. Build a dry-run plan. If root or listing is unknown, stop destructive operations.
3. Record `OperationIntent` in durable SQLite **before** any remote write; serialize expected guards, old baseline and file hashes.
4. Upload immutable blob/revision bytes with server-side `If-None-Match: *` (or a vetted equivalent). Bounded streams; for large files initiate multipart, send parts, conditionally complete, and abort incomplete uploads when safe. On existing key or ambiguous response, `HEAD -> GET -> BLAKE3 -> HEAD` before deciding whether this was our write. An ETag is never a plaintext digest.
5. Generate and publish a new immutable manifest with `If-None-Match: *`, verify its readback and store the expected hash. The parent link preserves the prior DAG heads.
6. Publish `refs/current.json` with `If-None-Match: *` for first creation or `If-Match: <observed ETag>` for a successor. On 409/412 or uncertain ACK, read back the current pointer and reconcile **without blindly replaying mutations**. Concurrent head proposals are replayed through a new planner and may become separate retained conflict branches.
7. Verify pointer identity, byte hash, manifest reference, and local journal state **before** advancing the confirmed baseline. Re-read manifest parent links if root moved during verification.
8. Download only into bounded verified staging, then let the Core File Gate enforce fresh local fingerprint, successful protective copy, and no dirty buffer at the actual local write. A provider client must never write directly into the vault.

Do not attempt remote GC in v1. No TTL-based pruning of blobs/revisions; absent offline devices may still reference them. Blob names for encrypted profiles must be redesigned before stage 06; never silently reinterpret this plaintext prefix as E2EE.

## Provider capability negotiation

- Real AWS `PutObject` and `CompleteMultipartUpload` support conditional create/replace; `DeleteObject` can be conditional, but remote destructive delete remains **disabled by Akula policy** in stage 05.
- **MinIO and alternative S3-compatible servers must pass live conditional semantics** before capability flags are enabled. A HEAD-before-PUT check is not an atomic substitute. If `If-Match` is absent/ignored, deny head publication and surface `PreconditionUnsupported`.
- `snapshot_consistency=true` is legal only when the manifest-backed snapshot implementation is verified. It is **false** for raw `ListObjectsV2`.
- `version_history=true` only if old provider versions can actually be retrieved, but Akula's immutable DAG retention is independent of server bucket versioning.
- Reject 401/403, 429, 5xx, invalid cursor, truncated page, lost auth, transient network failure and changed manifest as **unknown/inaccessible**, never a deletion.

## Credential, endpoint and resource policy

Start with MinIO `localhost` in test infrastructure, using throwaway credentials. Production endpoints must validate HTTPS, configured region, bucket and path-style/virtual-host mode; an explicit local emulator test exception may permit HTTP. Never disable TLS verification in production.

Use a least-privilege S3 identity scoped to one prefix: list that prefix, read/put immutable objects, read/write its current pointer and manage unfinished multipart uploads. No remote `DeleteObject` permission for stage 05, and no external writes to Akula-owned keys. Prefer a bucket policy enforcing conditional writes and versioning/retention protection where available; a policy is extra hardening, not a replacement for journal/readback.

Streaming should cap simultaneous transfers and in-memory buffers. `RemoteStore::read()` currently returns `Vec<u8>`; stage 05c must supply bounded/streaming semantics, including verified multipart readback, before moving large binary files. A new client must not treat unbounded `read()` as a safe large-file implementation. Do not insert a Python aioboto3 microservice.

## Small implementation slices (not a second top-level roadmap)

| Slice | Deliverable | Gate |
| --- | --- | --- |
| **05a — offline contract (this change)** | Rust S3 keyspace and head-pointer v1, strict decode/bounds/schema tests; explicit design and no network | `cargo test -p aquilum-core --locked` with and without `sync-engine`, native workspace and main gate; disabled default |
| **05b — transport/probe** | Rust S3 SDK client via internal worker, endpoint/credential validation, `HEAD`, bounded `GET`, `ListObjectsV2` repair listing, live probe of conditional create/replace | MinIO + AWS representative tests; fail-closed when semantics are not proven; no vault writes |
| **05c — immutable blob upload** | memory-bounded multipart/abort, exact readback hash, durable receipts and crash/lost-response reconciliation | 5 MiB boundaries, >64 MiB binary, duplicate key, 409/412/5xx/lost ACK, restart |
| **05d — manifest CAS and DAG transport** | immutable manifest body with verified references and stable logical pagination; atomic pointer publication; two-device CAS conflict replay | >1000 and 10k entries, two concurrent clients, incomplete list, root race/ABA guards |
| **05e — guarded two-client E2E** | connect S3 RemoteStore to PR 04 guarded local executor and durable SQLite baseline; two desktop instances and attachments | offline create/edit/rename/tombstone, delete-vs-edit, link/junction races, dirty buffer, repeated crash at each journal phase; zero unpreserved data |

**Prerequisite:** Stage 04 must finish the guarded local executor and cross-process recovery gates. Completing 05a or 05b is NOT permission to enable background sync or call local mutation code.

## Required release/CI proof

- Full `ListObjectsV2` pagination >1000 objects, exactly-once cursor handling for diagnosis; stable logical listing from pinned verified manifest, with reader detecting concurrent root advance.
- Two devices start from the same baseline, edit one file offline and both upload; both revision branches survive, neither uses mtime to destroy the other.
- Delete-vs-edit, rename-vs-edit, UTF-8/NFC and Windows path collisions, 0-byte and large binary blobs. Test S3 provider permissions, prefix escaping and path leaks.
- Send a successful server PUT but drop response; restart process; identify remote bytes with readback and journal rather than retry blind. Force CAS 409/412 and multipart timeout/abort.
- If the download's root/path/dirty buffer changes after plan creation, hold and preserve original content. Exercise cross-process symlink/junction replacement in actual File Gate.
- Cross-platform Windows/macOS/Linux full rust checks; real MinIO CI with isolated buckets + no live production credentials; separate AWS API compatibility gate before claiming AWS production support.
- No enable-by-default, no remote delete/garbage collector until separate reviewed retention + offline-device reachability protocol.

## Sources / related project work

- [Akula canonical PR 00–11](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11)
- [Akula S3 lessons from books_platform](https://telawiki.com/spaces/456/pages/7471/s3-books-platform-akula)
- [Notion sync direction](https://app.notion.com/p/3fb32a84beec81c1b561db8ad0b19df6)
- [AWS conditional write semantics](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html)
- [AWS conditional delete semantics](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-deletes.html)
- [AWS SDK for Rust: conditional multipart completion](https://docs.rs/aws-sdk-s3/latest/aws_sdk_s3/operation/complete_multipart_upload/builders/struct.CompleteMultipartUploadFluentBuilder.html)

## Implementation checkpoint: SYNC-05b (2026-10-17)

- [Draft PR #131](https://github.com/Lalalalendia/Akula/pull/131): real, **opt-in** AWS Rust SDK provider transport in `sync/s3_transport.rs` behind `sync-s3`. The default `sync-engine` feature does not start a network client.
- Rust `S3ConnectionProfile` requires valid bucket/region, HTTPS remote endpoint or explicit loopback-only HTTP for disposable MinIO tests. SDK ambient endpoint overrides are cleared when the application profile has no custom endpoint.
- `AwsS3ProbeClient` exposes only scoped `HEAD`, bounded streaming `GET` (1 MiB limit with HEAD / GET If-Match / HEAD verification), small server-conditional `PUT` and limited **diagnostic** ListObjectsV2 pages. No delete operation, no persistent secrets, no local file writes, no remote baseline commit and no `RemoteStore` registration. Snapshot consistency remains **unproven** until 05d.
- [MinIO run #39049772020](https://github.com/Lalalalendia/Akula/actions/runs/39049772020) verified 4 tests: endpoint guard, profile/ETag guard, server-side If-None-Match and If-Match including stale validation, bounded GET and diagnostics cursor handling. Final-head checks after hardening must still pass before merging.
- Old `quay.io/minio/minio` image now requires authentication; CI uses the free maintained `cgr.dev/chainguard/minio:latest` image with an isolated tmpfs owned by the unprivileged container user. Never treat the test server as a production S3 instance.
- Cargo.lock updated by a temporary branch-local resolver; the write-enabled generator workflow has been **removed** from the merge candidate. No CI workflow needs repository write permission for S3 tests.
- Still outstanding for full 05b: AWS S3 account E2E using narrowly scoped temporary credentials and provider compatibility matrix (e.g. 403/429/5xx); full 05c requires multipart/abort, stream-to-disk staging, and durable operation receipts. Do not enable bidirectional sync before PR 04/05d/05e acceptance.
