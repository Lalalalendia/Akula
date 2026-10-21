# YC-S3-T01d E3g — integrated receiver genesis and publisher successor lab

**Opt-in only. No default or installed Akula activation. No real Yandex.**

## Purpose

E3e [PR #503](https://github.com/Lalalalendia/Akula/pull/503) issues and spends a one-shot receiver challenge before invoking E3c atomically signed genesis enrollment. E3d [PR #500](https://github.com/Lalalalendia/Akula/pull/500) permits later V2 root progression only by the currently stored publisher quorum. Previous PRs tested the two paths separately, not against the **same persistent SQLite root+policy row**.

This stacked laboratory branch includes both original implementations under the explicit feature `signed-vault-join-successor-integration-lab` (dependent on `signed-vault-receiver-session-lab` and `signed-vault-signed-successor-lab`).

## Integrated test

`one_shot_receiver_join_then_signed_direct_successor_preserves_same_sqlite_policy`:

1. The receiving device obtains a 32-byte OS CSPRNG challenge, with five-minute monotonic session expiry and scope binding.
2. An independently described owner-pinned synthetic DSSE invitation passes E3a checks and synthetic complete V2 read checks. E3c inserts generation-1 root **and** owner/publisher policy atomically.
3. The consumed session rejects a replay before consulting the SQLite journal.
4. An unauthorized publisher cannot promote generation 2 even with correct direct ancestry and synthetic pinned content.
5. An authorized publisher advances generation 2 in the existing SQLite row. After closing and reopening the database, the exact generation and immediate predecessor remain, and the policy digest is unchanged.
6. The legacy unsigned V2 promotion API still HOLDs even after a signed successor was accepted.

The Linux+Windows exact-head CI first lists and requires E3e one-shot, E3d distinct 2-of-2 quorum and the new integrated test name. It then runs that exact integration test and all related V2 policy, journal and owner-invitation regressions. Zero-test false greens are blocked.

## Boundaries and next required gates

- This uses a *test-only synthetic pinned V2 receipt*: it does not bind a real network GET to the owner-approved keyset. Separate [E3f #506](https://github.com/Lalalalendia/Akula/pull/506) exercises real MinIO HTTP/SigV4, not production Yandex identity.
- The caller-supplied owner keys and root/policy digests do not prove human out-of-band approval. The original E3c research method remains accessible: the one-shot wrapper is not enforced by native application UI.
- The independent witness against full DB rollback / S3 freeze and equivocation remains missing.
- Windows CredUI #428 still needs actual interactive Windows UIA masked field/Cancel acceptance.
- A true user-initiated read-only Yandex probe requires private user-approved disposable bucket and least-privilege credentials; no live Yandex call runs here.
- Existing PUT/CAS transports remain test infrastructure; no writer is activated by this feature; no File Gate or user Markdown changes and no automatic Sync.

Source RFC: [YC-S3-T01d](https://app.notion.com/p/3ff32a84beec813ebfdff9b96bf103d1).

**Acceptance:** exact-head Linux+Windows CI must execute a nonzero number of E3g cases with all assertions passing. Keep Draft until then.


## E3g network-to-trust handoff (21 October)

The previous E3g test uses `S3PinnedV2ReadPreview::for_trust_anchor_test`, a synthetic receipt. This is adequate for SQLite and cryptographic transitions but cannot authorize a production enrollment. The latest E3g branch adds a **separate internal, read-only entrypoint**:

`ReceiverJoinSession::enroll_from_yandex_read_once(client, journal, scope, root, policy, signed_offer, independently_approved)`

1. The receiving device *spends* its existing CSPRNG challenge **before** any cloud request and validates the original device/profile/scope and independently received challenge.
2. The E3b implementation verifies the owner's DSSE invitation against the externally supplied trusted keys and exact approved root/policy BLAKE3 fingerprints **before** contacting a provider.
3. A precisely Yandex-bound `AwsS3ProbeClient` reads the expected root and its observed ETag. The existing bounded V2 inspector then verifies canonical root bytes, manifest, index, packs, DAG ancestry and a final root re-read using actual HTTP/SigV4 GET.
4. Only an internal `pub(crate)` handoff receives the exact `S3PinnedV2ReadPreview` and ETag, and passes them straight to E3c's atomic root+policy SQLite enrollment. The diagnostic public `preview_yandex_join_genesis_read_only` still returns **only the summary**, not reusable authorization evidence.
5. A failed signature, wrong scope, timeout, corrupt object, account-binding discrepancy or SQLite failure consumes the ephemeral challenge. A new receiver challenge is required to retry.

An explicit adversarial async test uses a real `AwsS3ProbeClient` SDK constructor but **no S3 call**, requiring forged owner evidence to be rejected locally and a retry to yield `AlreadyConsumed`. Its presence and execution are mandatory in Linux+Windows E3g CI.

**This code path has NOT been exercised against a real user Yandex account or a complete real V2 object graph.** A successful negative test and MinIO reader test from E3f are separate evidence, not a combined live Yandex PASS. There is still no user-visible authenticated approval/QR screen, independent Yandex principal attestation or external monotonic rollback witness. Release remains blocked, no File Gate, no S3 PUT/CAS/DELETE and no automatic Sync.


## Operator-owned Yandex acceptance prerequisites (not yet executed)

A future **user-authorized** acceptance must use a disposable/private Yandex bucket, a fresh service account restricted to that bucket with `storage.viewer` (not `storage.editor` or `storage.uploader`), and credentials supplied only to the installed host/SDK credential provider, never GitHub Actions logs, Notion or this repository. Yandex documents that `storage.viewer` can read bucket objects and metadata while the writing roles grant uploads or deletion. The test must verify the exact intended `https://storage.yandexcloud.net` endpoint and independently confirmed owner keys and genesis/policy digests. A viewer account cannot seed a V2 fixture; that must be provisioned separately by the owner before read-only acceptance.

Acceptance evidence: (a) exact challenge consumed once, (b) signature and owner policy verified off-S3, (c) real Yandex HEAD/GET complete V2 content and final root re-read, (d) atomic *local app-data-only* root+policy registration, (e) denial under incorrect key, bucket, pin, ETag, revoked publisher or missing credentials, and (f) no Object Storage PUT/DELETE, File Gate or note mutation. Record only sanitized result codes, non-secret provider/bucket identifiers and test counts.

Official role reference: https://yandex.cloud/en/docs/storage/security/ and bucket-scoped role assignment: https://yandex.cloud/en/docs/storage/operations/buckets/iam-access.
