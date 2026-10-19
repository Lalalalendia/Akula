# STY-G3 / RN-4B — signed peer/object gate (APP-GRANT-01)

**Status:** isolated laboratory contract, **not production authorization or sync**. Stacked on [FILEGATE-RECEIPT-02 PR #335](https://github.com/Lalalalendia/Akula/pull/335); no changes to real Akula Sync Core/File Gate, native app or installed files.

## Why this is not another transport or File Gate

- [Staya/Roha transport PR #379](https://github.com/Lalalalendia/Akula/pull/379) already proves admitted QUIC peers and denied unknown network identities on Linux/Windows/Android API35.
- [FILEGATE-RECEIPT-02 PR #335](https://github.com/Lalalalendia/Akula/pull/335) already exercises the *actual* `aquilum_core::files::gate::create`, file/directory sync and independent restart/readback. It correctly does not claim a remote authorized commit.
- This narrowly tests the **missing boundary** between them: a cryptographically authenticated connection does **not** authorize a particular DeviceId, VaultId, object or operation. A valid Ed25519 grant is still insufficient without *current locally trusted Akula policy*.

## Test contract

The new `peer_scope` module only accepts **Pull** of a precisely bounded object. A grant signature covers a domain-separated fixed layout containing:
1. QUIC `EndpointId` bytes **from `Connection::remote_id()`** (not a remote-supplied identity claim), Akula DeviceId, VaultId;
2. expected BLAKE3 root and exact length (lab limit 4 MiB);
3. current policy epoch, grant expiration and locally expected nonce;
4. the operation byte.

The grant is independently verified with an app-provided trusted issuer key. A local `LocalPolicy` must also bind the exact endpoint, device, vault, hash, size, generation and nonce; missing, revoked, expired or stale is **denied**. The existing file-gate fixture identifies destinations using a numeric `tag`. The local policy now **also pins that tag** so a correctly signed grant cannot be replayed into another test workspace. This tag is **not** a production VaultId-to-path identity or an OS inode custody proof. Signature alone never enrolls a peer or creates a grant.

Negative tests exercise: mutated/wrong signature and issuer key; unregistered or revoked peer; authenticated endpoint mismatch; device/vault mismatch even under a *valid* signature; **cross-workspace destination tag despite a valid signed grant**; stale epoch; nonce mismatch; expired grant; absent or mismatched object; oversized object; properly signed PUSH and DELETE. Every negative gate test asserts that there is **no created user file and no receipt**.

A positive **fixture-only** path calls the **existing** real File Gate function to create one fixed, known local note. It explicitly confirms that its network grant and file readback **do not** produce a replica/durable receipt, and second publication cannot overwrite. The existing signed receipt and crash/reopen studies remain unchanged.

## Limitations and next real prerequisites

- No user-vault files from QUIC are accepted or written. Input bytes in the positive case are the locally defined known `TEXT` fixture, not network content. The legacy fixture-only `filegate_publish` is still callable from other research tests: this is **not** a production-enforced entry point.
- The test signer seed is hard-coded by design; never reuse for production. There is no canonical grant serialization/wire decoder, enrollment, attested issuer custody, atomic persisted Core snapshot, monotonic anti-replay service, real-time clock guarantee, signed revocation history or hostile writer protection. All must be implemented under the existing Core's ownership before the policy is authoritative.
- No `APPLIED`, ACK, durable staged transfer or `VerifiedReplicaReceipt::new_prevalidated` can be inferred from a passing gate. Existing FILEGATE-RECEIPT-02 is a **separate** local-disk laboratory and not automatically linked to the remote.
- Current P2P-BLOB-11a filesystem/Bao custody research remains separate; source of truth for production writes stays Akula File Gate, not the Roha/noq adapter.
- Feature flag **OFF**, no sync schedule, no automatic pairing, no namespace publication, no new file mutation capability, no change to production `main`.

## Promotion gates

1. Verify new deny-before-mutation tests and all existing FILEGATE-RECEIPT-02 tests on exact PR head (Linux host only; no Windows durable-parent claim).
2. Bind `Connection::remote_id()` to persisted, revocable Core-owned DeviceId/VaultId grants, without permitting network-origin policy injection; reject previous epochs after revocation/restart.
3. Authorize only an exact signed object manifest. Verified network bytes must pass Bao/staging and collision policy before any Core File Gate call.
4. Real Core File Gate durable readback plus a host-owned signed, non-replayable receipt is required before `APPLIED`. Crash/SIGKILL before receipt must leave `NeedsReview`, never mark baseline confirmed.
5. Independent Windows/Android and actual two-device loss/restart/rollback acceptance before any merge or feature enablement.

Source plan: [STY-MASTER-PLAN-02](https://app.notion.com/p/3fe32a84beec81e5acf7c081fe5e0bfd).
