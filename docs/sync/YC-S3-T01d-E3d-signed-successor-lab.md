# YC-S3-T01d E3d — signed V2 direct-successor admission laboratory

**Research-only, opt-in, no default Akula activation.** Feature `signed-vault-signed-successor-lab` depends on E3c. It introduces no S3 writer, File Gate permission, downloader into a user workspace, background scheduler, new trust DB, or credential persistence.

## Problem

E3c correctly refuses advancing an enrolled signed Vault via the older `record_verified_successor` API, but before E3d there was no separate authorized way to record a generation-2 successor. A complete V2 pinned-read proves object consistency, **not** that the current publisher approved the new root.

## E3d admission

The opt-in `record_verified_signed_successor` validates:

1. A complete, bounded existing `S3PinnedV2ReadPreview` for the exact candidate root, remote scope and observed ETag. That preview is minted by the guarded pinned reader, not by the transport's LIST success. ETag detects an object change; it never grants identity.
2. The previous trusted V2 root and the **current** owner/publisher-policy receipt are loaded from the same guarded private SQLite database under **one BEGIN IMMEDIATE transaction**.
3. The candidate must be the exact **direct successor** with generation N+1 and the expected previous V2 root digest. Same-generation fork, rollback and skipped generations fail closed.
4. DSSE PAE / Ed25519 signatures are checked over authenticated candidate bytes with the **CURRENT stored** publisher key set and distinct-key threshold, including exact bucket, VaultId, keyspace/current-head path, Yandex provider identity and policy epoch. Cloud-supplied signers cannot select trusted keys.
5. Once verified, **only the existing V2 row's root, ETag, predecessor and private receipt** advance atomically. The policy stays in the same row and is guarded by a compare-and-swap on policy digest/receipt and prior root receipt.
6. Re-reading the same signed candidate and same ETag can return `Unchanged` only if the current policy still authorizes its signature. A rotation may intentionally make an older signature inadmissible even for a no-op.

The original `record_verified_successor` remains blocked for enrolled policy-bearing Vaults even when the E3d feature is present. No new trust authority is created by a legacy API, and pure root-only research journals keep their existing separately restricted behavior.

## CI and adversarial tests

Linux Ubuntu 24.04 + Windows Server 2022:

- Signed generation 1→2→3 persists across restart with the exact previous root and unchanged policy digest.
- Correct signed replay with unchanged ETag is idempotent.
- Wrong publisher, revoked publisher, stale policy epoch and wrong ETag refuse promotion.
- A rotated owner/publisher policy from a second SQLite handle takes effect before signature authorization; a previously permitted signing key cannot advance the root.
- Competing handles cannot silently accept a same-generation signed sibling fork; the legacy unsigned path still HOLDs.
- A pinned root with no publisher policy cannot use the new method as implicit enrollment.
- E1/E2a/E2b/E3a/E3b/E3c regressions plus default Core and E2b-only downgrade checks.

All tests use disposable app-data, synthetic Ed25519 keys, and test-only pinned receipts. **No real remote Yandex object is contacted.** Successful CI is proof of these laboratory contracts, not proof of live security, user-approved key enrollment, S3 principal identity or global freshness.

## Unsolved production gates

- True independently authenticated owner-key approval and challenge lifecycle, including CSPRNG, expiration and replay protection.
- Real Yandex read-only S3 HEAD/GET against user-authorized credentials, including an end-to-end pinned manifest/index/pack gate.
- Persistent authenticated evidence/witness against complete malicious rollback of the local SQLite DB and S3 split view or freeze.
- Audited bounded DSSE JSON/base64 wire decoder and stable cross-language canonical protocol.
- Windows Credential Manager/CredUI QA in #428 and installed app integration.
- A complete authority design for the future writer/Sync remains separate. **Do not enable S3 PUT/CAS/DELETE, File Gate, automatic Sync, or mutate user Markdown.**

Stack: [E1 #472](https://github.com/Lalalalendia/Akula/pull/472) → [E2a #475](https://github.com/Lalalalendia/Akula/pull/475) → [E2b #481](https://github.com/Lalalalendia/Akula/pull/481) → [E3a #487](https://github.com/Lalalalendia/Akula/pull/487) → [E3b #491](https://github.com/Lalalalendia/Akula/pull/491) → [E3c #493](https://github.com/Lalalalendia/Akula/pull/493).

[Source Notion security RFC](https://app.notion.com/p/3ff32a84beec813ebfdff9b96bf103d1)
