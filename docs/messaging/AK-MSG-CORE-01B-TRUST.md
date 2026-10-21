# AK-MSG-CORE-01B: sealed trusted DeviceId-WOW signing key registry

This is an opt-in *Akula Core* laboratory, not a second account service or
production enrollment path. Stacked on PR 537; feature
`messaging-key-ledger-lab` is **off by default**.

## Evidence and permissions

- W4TrustLedger provides a crash-durable SQLite WAL/FULL mapping from Akula's
  existing typed sync::DeviceId to an Ed25519 verification key for a single
  strict trust epoch. It implements the existing W4 TrustedRecipientKeys port.
- Monotonic epoch rotation requires a distinct host-authorized enrollment;
  same-epoch key replacement, rollback and revocation revival are refused.
- Revocation survives restart, blocks previously signed W4 receipts and
  cannot be silently undone in the same epoch. Unknown devices stay unknown.
- The ApprovedEnrollment / ApprovedRevocation structs have **private fields**
  and **no normal constructors**. The tests build them inside their test
  module; an actual authorized Akula invitation ceremony must be developed
  before anything can enroll a real external key. It is NOT trust-on-first-use.
- Enrollment evidence digests are bookkeeping only, **not cryptographic proof
  by themselves**. The privileged host must authenticate original evidence
  before creating either sealed object.
- This does not create a second Sync Core, a standalone messaging product or
  file access grants. A verified device signing key alone NEVER authorizes
  reading a user's Markdown files or workspace.

## Remaining security blockers

- No actual QR/invite or authorized multi-device key exchange, confirmation UI,
  installed-app identity authority, protected root of trust or OS-backed
  private-key storage. The trusted SQLite file must live in guarded app-data;
  an attacker able to replace it may roll back or forge its contents.
- Separate trust lookup and sender/recipient SQLite writes are NOT protected
  by one atomic revocation fence. This blocks security-sensitive deployment.
- No actual W3 authenticating decrypted envelopes, durable real connected
  network chat, or Bluetooth/mesh implementation.
- A newer host-approved epoch is allowed after revocation; replayed old signed
  ACKs remain untrusted. Physical custody, audit and rotation policy are still
  separate requirements.

## Reproduce

From `aquilum-app`:
`cargo test --locked -p aquilum-core --features messaging-key-ledger-lab 'messaging::'`
`cargo check --locked -p aquilum-core --no-default-features`

Spec: https://app.notion.com/p/40032a84beec81919ec8df2e31056543
