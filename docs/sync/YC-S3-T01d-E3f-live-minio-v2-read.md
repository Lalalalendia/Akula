# YC-S3-T01d E3f — real S3 GET of a complete pinned V2 object graph

**Test-only MinIO integration, NOT a live Yandex acceptance or production authorization.** Built on E3c research branch, but exercises the existing production-compiled `AwsS3ProbeClient::inspect_explicitly_pinned_v2_read_only` directly without signed-key enrollment.

## Why

Earlier E3a/E3b/E3c CI used signed offline invitations and `S3PinnedV2ReadPreview::for_trust_anchor_test`, a test-only synthetic receipt. That cannot prove the SDK actually fetched and verified a complete network object graph. E3f puts a real S3-compatible server in the loop.

## Fixture and acceptance

A **disposable, loopback-only** MinIO container receives six explicitly constructed synthetic V2 objects using test credentials and the existing SDK's guarded conditional-write API:

- One `AKPKEXP1` pack containing one synthetic Markdown revision and BLAKE3-verified content.
- Immutable manifest shard and manifest root.
- Immutable V2 pack-index shard and index root.
- Mutable `refs/current.json` containing a canonical generation-1 V2 root; exact observed ETag is pinned **only for this experiment**.

Fixture upload happens **before** measurement. Then the production-compiled pinned reader must:

1. Perform real S3 GETs for the root (initial and final), manifest, index, shards and pack.
2. Reject swapped profile/account scope, wrong root/ETag, or changed immutable pack bytes.
3. Return exactly one verified revision and pack with a valid V2 proof bound to the scope and root/ETag.
4. Reject a replaced current-head object rather than treating it as a new trusted root or empty vault.
5. Call **no PUT, DELETE, File Gate, synchronization or note-writer operation** during the read phase. Test fixture writes are restricted to disposable MinIO.
6. Require one explicit `#[ignore]` test name and fully qualified filter in workflow, guarding against false-green zero-test CI.

CI: [YC-S3 E3f real MinIO V2 pinned read](https://github.com/Lalalalendia/Akula/actions/workflows/yc-s3-e3f-live-minio-pinned-v2.yml) on Ubuntu 24.04. Normal no-feature Core compilation remains a separate step. All test credentials are static disposable values, unrelated to any user's Yandex account.

## Honest limits

- MinIO tests a compatible S3 HTTP/SigV4 operation, **not Yandex's real endpoint, region, TLS/credential custody, SLA or account identity**.
- The synthetic pack is real network data for the inspector, but **the signed owner invitation, owner-approved keys, one-time challenge, SQLite genesis commit and a complete cross-device flow are not exercised together here**.
- A valid ETag is a concurrency token, not a signature. Global latest, server split-view/freeze and malicious whole-SQLite rollback require independent witnesses and threat-model decisions.
- No shipping Akula setting or automatic Sync is enabled. Real private Yandex read-only tests need a separately user-authorized disposable bucket and credential path; do not use production buckets or list/read private content without explicit approval.

Source: [YC-S3-T01d security RFC](https://app.notion.com/p/3ff32a84beec813ebfdff9b96bf103d1), dependency [E3c #493](https://github.com/Lalalalendia/Akula/pull/493).
