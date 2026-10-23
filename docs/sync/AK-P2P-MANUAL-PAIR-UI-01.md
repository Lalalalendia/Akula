# AK-P2P-MANUAL-PAIR-UI-01 — protected, manually verified peer pin in File transfers

Status: **opt-in only**, stacked after #572 (real Credential Manager local
identity and peer pin/revoke) and #606 (real local Iroh public ID in the native
File transfers window). This is not a working network transfer.

## First user-facing pairing action

1. The first Akula shows **its own public EndpointId** through #606, after
   explicit read/first-run enrollment. No secret seed or private-key material
   leaves Windows Credential Manager.
2. The operator receives an expected 64-character lowercase Iroh EndpointId
   for the OTHER device (for example, from its future connection invitation).
   Enter it in **Observed EndpointId**. The invitation by itself is untrusted.
3. The operator must independently check the **full 64 characters** on the
   other device's own Akula screen, in person or via a separately authenticated
   human channel, and enter that result into a SECOND non-prefilled text field.
   Do not auto-copy values between the two fields. A mismatch, malformed ID,
   all-zero ID or a missing protected local identity is rejected, not stored.
4. Explicit **Confirm** calls the existing Core `peer_pin::confirm_peer` on a
   background worker and independently re-reads `peer_pin::peer_state` before
   displaying the persistent OS status. Duplicate enrollment cannot replace
   an existing record.
5. Explicit **Inspect** reads the fresh protected record. For a confirmed
   key, **Request revocation** opens a distinct warning showing the target ID.
   A separate confirmation is required to execute `revoke_peer` on a worker;
   the resulting tombstone must be read back as Revoked. Cancellation is inert.
6. A generation fence rejects stale async results, and an edit invalidates a
   previously inspected state or armed revocation. A changed key cannot be
   revoked via a stale button. The UI remains responsive during OS calls.

## Critical security boundaries

- Manual pin means **expected remote Ed25519 key**, NOT an authenticated
  active connection. Even a confirmed OS record does NOT grant file access,
  provider-side Staya GET, Vault permissions, or a network send. The screen
  says this in Russian and English.
- This manual user-verification procedure assumes the second field comes
  from an independent, trustworthy observation of the other device. If both
  values come from the same untrusted message, this is NOT verified pairing.
  The UI cannot technologically verify which human channel was used.
- Before every subsequent network operation Akula MUST call the existing
  `require_pinned_remote(expected_id, actual_iroh_remote_id_bytes)`. Those
  remote bytes must originate in an established authenticated QUIC connection,
  NEVER from invitation text. Revocation applies to subsequent operations
  and is not retroactive cancellation of already received bytes.
- Do not use a `DeviceId` from a merely observed profile as proof of OS
  peer pin. No implied mapping to the Vault identity or owner-signed ACL.
- The regular installed release does not enable `p2p-device-ui`. There is
  no plaintext credential fallback and no separate registry database.

## Validation and non-goals

New pure UI/state tests prove separate input widgets, command availability,
mismatch failure before OS mutation, required two-click permanent revocation,
changed-target cancellation, bounded IDs, and stale-generation filtering.
The Windows workflow compiles the actual native UI with the feature and runs
these tests and #606 identity tests. Existing #572 independently tests
Credential Manager persistence and revocation across OS processes.

Remaining true user E2E: authenticated QUIC peer observation and owner-visible
approval; bind exact Akula OfferSession to provider-side #79 Staya permission;
actual sender/receiver process lifecycle in File transfers; Bao/BLAKE3 receiver
inbox; Save as New from #571; installed two-PC acceptance.

Security release gate #595 (RustSec plus external AWS credential-process
regression) remains mandatory; do not bypass it for these features.
