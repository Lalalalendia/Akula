# YC-S3-T01d E3b — owner-signed genesis plus complete pinned S3 read

**Status: research-only opt-in experiment.** Feature: `signed-vault-pinned-join-lab`. Depends on E1/E2a/E2b/E3a, includes S3 client only when explicitly compiled. Standard/native Akula does not enable it.

## Why this experiment exists

An Ed25519 owner signature alone does not prove that the actual Yandex bucket contains the exact genesis root and complete immutable contents. An S3 LIST/HEAD/ETag alone does not prove who owns the Vault. E3b requires **both independently** and rejects partial success.

## Verified boundaries

1. Receive **independently authenticated**, human-reviewed E3a expectations: expected Yandex bucket, full VaultId/keyspace/current-head key, owner public keys+quorum, **separate approved genesis root BLAKE3 and policy BLAKE3**, and a fresh recipient challenge. These must **NOT** originate solely from S3. E3b does not implement human pairing/approval or generate the challenge.
2. Verify a DSSE/Ed25519 **owner-quorum signed genesis invitation** using *only* those independent expected keys and pins; E3a refuses foreign scope/keys, changed challenge, generation >1 and policy epoch >1. E3b calls E3a *before network*.
3. Require the **actual selected S3 profile** to match the *signed Yandex identity*: exact `https://storage.yandexcloud.net` endpoint, `ru-central1` region, path-style mode, approved bucket, and keyspace/VaultId. The authenticated SDK client must match that same scope. Another HTTPS S3 provider or region is **not** acceptable just because it has the same bucket and object names. No default bucket scan or cross-profile fallback.
4. Explicit call to `preview_yandex_join_genesis_read_only` (never automatically scheduled) performs a **bounded GET of exactly the approved current-head key**. The bytes must equal the independently approved canonical V2 root. The returned ETag is an **untrusted concurrency token**, never proof of publisher identity or content integrity.
5. Reuse existing `AwsS3ProbeClient::inspect_explicitly_pinned_v2_read_only` with that exact genesis and ETag. This reads exact root, immutable manifest/shards, index, pack bytes and verifies BLAKE3/complete DAG; it **re-fetches the same root at the end**, refusing ETag/content changes and malformed/foreign/incomplete pack history.
6. `inspect_pinned_join_receipt` checks the **full pinned reader's internal evidence** (scope fingerprint, exact canonical root digest and ETag, `requires_explicit_file_gate_review`) against the *same* owner-reviewed root; result is a typed, **read-only** `PinnedJoinReadOnlyObservation`, not any write permit or saved trust.

No second SQLite source of truth, no trust journal mutation, no File Gate write, user note modification, S3 PUT/DELETE/CAS, scheduler, baseline promotion or automatic Sync.

## Tests and evidence

`cargo test --locked -p aquilum-core --no-default-features --features signed-vault-pinned-join-lab s3_join_pinned_lab::tests`

9 offline tests using synthetic owner keys and the **test-only** pinned-read receipt factory validate:

- Signed externally approved genesis + corresponding fully pinned exact V2 evidence → read-only observation.
- Missing/changed/invalid ETag and mismatched complete-read evidence → HOLD.
- Wrong bucket/profile/VaultId/keyspace, foreign HTTPS endpoint, region or addressing mode → HOLD.
- Another root, even with a valid reading receipt, cannot reuse a signed invitation.
- Complete root/pack evidence does not substitute a trusted owner signature.
- Correct owner signature does not substitute complete pinned read.
- Later generation cannot be silently enrolled using signed genesis.
- A different recipient challenge cannot replay an otherwise signed invitation.

The CI on Windows 2022 and Ubuntu 24.04 runs E3b, E3a, E2a, E2b regression and default Core checks. **The test-only receipt cannot prove live Yandex behavior**; that remains a separate user-authorized provider gate. No real cloud credentials are stored or transmitted in CI.

## Remaining release gates

- Actual separate Windows CredUI QA and live private Yandex read-only test with locally entered credentials and user-approved test bucket; no automatic connection or cloud resource provisioning.
- Production-safe DSSE JSON/base64 wire decoding with bounded and independently reviewed parser; stable canonical signing/policy wire format.
- Human-visible out-of-band key/channel verification and one-time challenge storage/consumption. Merely passing a string to the E3a API does not create real human approval.
- Integrate E3a/E3b with the **existing** guarded V2 root + policy journal only once independent approvals and complete read evidence exist; design atomic consent+root+policy enrollment and restart. Never make a successful review automatically enroll a remote root.
- E4 independent witness/fork checks for malicious **whole-SQLite rollback**, hidden newer generations and S3 split-view.
- File Gate / S3 CAS writer admission separate and **not authorized by this PR**.

Source RFC: https://app.notion.com/p/3ff32a84beec813ebfdff9b96bf103d1

**No production-ready claim.** A successful offline E3b test proves only that owner invitation review and a complete pinned V2 receipt can be matched safely, not that an external provider/human identity/pairing was verified.
