# YC-S3-T01d E3a — offline, owner-signed genesis join review

**Scope:** fully offline, opt-in `signed-vault-pairing-lab` experimental Core verifier. Stacked on E2b, but the E3a feature depends only on E2a (no AWS SDK). **Does not read Yandex S3, touch user files, generate or import signing keys, establish Windows CredUI, enroll SQLite, or grant Sync/File Gate/S3 PUT authority.**

## Why this is necessary

A successful Yandex SigV4 LIST proves S3 permission only. A BLAKE3 hash returned by S3 is not independent evidence of the Vault's identity. An Ed25519 signature is useful **only if the recipient obtained the owner's verification keys through an authenticated channel independent of S3**. Even a correctly signed history from the remote server may be stale/forked.

E3a is deliberately limited to **generation 1 and policy epoch 1**. It cannot silently adopt an arbitrary current head for an existing vault. Joining a later generation needs an authenticated genesis-to-current ancestry and the existing pinned-read V2 trust journal (future E3b).

## Proposed receiving-device and owner-device flow

1. Receiving device independently obtains and explicitly approves the expected Yandex bucket+keyspace/VaultId, exact genesis V2-root BLAKE3, genesis owner-policy BLAKE3, full authorized Ed25519 owner keyset/quorum. It must obtain this through an authenticated external channel, not the bucket/README/listing provided by the untrusted S3 endpoint.
2. Receiving device produces an unpredictable **32-byte challenge with an operating-system CSPRNG**; the code expects 64 lower-case hexadecimal characters and refuses all-zero values. E3a itself intentionally has **no** RNG or UI. A challenge must not be reused. The pairing UI and challenge delivery to the owner are **not implemented**.
3. Owner(s) construct a canonical JSON `join-genesis` review payload binding: schema/purpose, Yandex endpoint+region+path-style, bucket, exact keyspace current-head key, VaultId, genesis root digest, epoch-1 policy digest, and recipient challenge.
4. Owner(s) sign **exact payload bytes** with DSSE v1 PAE and the required *distinct owner-key quorum* using Ed25519. Key IDs in the DSSE envelope are untrusted hints.
5. Recipient verifies signatures using **only the pre-approved owner keys**, validates both independent BLAKE3 pins, entire authority keyset+threshold, expected scope, purpose and recipient challenge. Only then may it display a typed `JoinGenesisReview`.
6. **Stop:** a positive review is not a persisted trust relationship. Separate consent, remote pinned V2 reads of root/manifest/index/ETag, admission to the guarded trust journal and anti-rollback/witness are future gates. No S3 upload, user-file apply, baseline promotion or background sync occurs.

## Security and operational boundaries

- `DecodedDsseEnvelope` is input: E3a **does not** implement an untrusted JSON/base64 envelope codec. A future network/UI adapter needs its own size/ambiguity/fuzz review.
- All positive results are read-only. The invite itself is not a capability; a public key fetched from S3 cannot authenticate itself.
- Expected genesis pins are **both** independently approved and compared against the supplied in-memory V2 root and `PublisherPolicyV1`.
- Policy's owner set and quorum must equal the externally approved owner set/quorum. A forged keyid, a foreign signer or duplicate signatures cannot increase authority. Owner keys must be nonzero and distinct.
- The fresh challenge bounds replay **only if** the caller uses an unpredictable, single-use challenge and an authentic owner signing flow. E3a does not prove global freshness or implement challenge replay storage.
- Explicitly rejects generation >1, policy epoch >1, foreign bucket/namespace/VaultId/provider, incorrect root/policy hashes, missing signatures, malformed payload or unknown/duplicate signed JSON fields.
- No canonical cross-language invite serialization or key signing/custody is frozen by this experiment.

## Tests and CI

`cargo test --locked -p aquilum-core --no-default-features --features signed-vault-pairing-lab s3_join_genesis_lab::tests -- --test-threads=1`

Twelve groups: valid 1-of-1; 2-of-2 and duplicate signing; unknown/foreign owner key; wrong externally pinned root/policy; wrong bucket/prefix/endpoint/region/VaultId; wrong challenge/zero challenge; payload type/forged keyid/tampered signature; signed unknown/duplicate fields; duplicate authority/zero quorum; generation or epoch >1 refusal; no signatures/wrong root; oversized payload. Synthetic test keys only.

Dedicated `.github/workflows/yc-s3-e3a-offline-pairing.yml` covers Linux and Windows plus E1/E2a regressions and default Core check. No new Cargo dependencies; normal installed Akula does not enable this feature.

## Pending work

- E3b: independently trusted owner onboarding UI, signer custody, verified wire envelope parsing, real pinned read of Yandex V2 genesis root+manifest+index and secure user authorization to enroll in existing guarded journal.
- E3c: proofs of ancestry and trusted publisher policy for existing generation >1; no TOFU.
- E4: independently witnessed monotonic checkpoints against complete local DB rollback and malicious cloud fork/freeze.
- [Windows/Yandex read-only PR #428](https://github.com/Lalalalendia/Akula/pull/428) physical CredUI and authorized real private-bucket test are separate blockers.

**Draft/HOLD**. A successful offline test does not prove remote ownership, remote freshness, human consent or safe Sync.

Related: [Notion YC-S3-T01d RFC](https://app.notion.com/p/3ff32a84beec813ebfdff9b96bf103d1), [DSSE protocol](https://github.com/secure-systems-lab/dsse/blob/master/protocol.md), [TUF](https://theupdateframework.github.io/specification/v1.0.36/).
