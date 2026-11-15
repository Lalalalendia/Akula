# AK-MSG-CORE-01B/C: signed WOW W4 receipt and durable sender outbox

**Status:** opt-in experimental components *inside Akula Core*, not a separate
messenger. This stacked branch builds on
[CORE-01A PR #530](https://github.com/Lalalalendia/Akula/pull/530).

## Existing WOW work reused

Wire: https://github.com/calex794/wow/blob/main/src/receipt_w4.rs
Journal semantics: https://github.com/calex794/wow/blob/main/src/journal_w4.rs

- Binary-compatible signed recipient receipt (137 bytes): version, DeviceId
  bytes, message ID, BLAKE3 ciphertext digest, trust epoch and Ed25519 signature.
- Real Ed25519 verification via the existing Akula ring dependency.
  An independently generated cryptography-library signature vector is tested.
- Current trusted key must be provided for an exact DeviceId+epoch by an
  *independently authorized* Akula identity service. Unknown or revoked keys
  fail closed. There is no permissive auto-enrollment implementation.
- SQLite WAL and synchronous FULL; IMMEDIATE transaction, identical retry bytes,
  bounded packet (16 KiB), immutable message ID/digest, restart-safe state and
  idempotent final recipient acceptance. Relay-only storage is NEVER delivery.
- Existing pure state machine accepts the same real signed W4 wire via W4Verifier.

## Limitations

This is a compatible adapter using existing ring/rusqlite, not a direct Cargo
dependency on WOW yet. The caller must ensure input is a genuine encrypted,
authenticated WOW W3 envelope; this adapter cannot detect plaintext. The
recipient's durable W4 inbox, signer-after-commit, key enrollment, independent
revocation storage, atomic authority-epoch fence, live Roha/noq/BLE transport,
contacts, UI and File Gate attachments are *not* implemented here. Do not merge
or enable for normal users before cross-crate integration and security review.

## Reproduce

From aquilum-app:
- cargo test --locked -p aquilum-core --features messaging-w4-lab messaging::
- cargo check --locked -p aquilum-core --no-default-features
- rustfmt --edition 2021 --check core/src/messaging/mod.rs

Spec: https://app.notion.com/p/40032a84beec81919ec8df2e31056543
Simulation: https://app.notion.com/p/40032a84beec8113a9e0ddd204081f50
