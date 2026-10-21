# AK-MSG-CORE-01C (receiver): durable W4 inbox in Akula Core

**Status:** experimental, stacked on PR 537 (which itself depends on PR 530).
The built-in Akula messaging feature is NOT yet available to users.

The opt-in `messaging-w4-inbox-lab` layer contains a bounded-size SQLite
receiver inbox, W4 Ed25519 receipt creation, exact replay and deduplication,
and a write-failure test. It uses existing Akula dependencies and the
previously implemented WOW W4 receipt verifier.

**Fail-closed trust boundary:** `AuthenticatedW3` is unconstructible by
ordinary callers: only the future independently reviewed WOW W3 decrypt+
authenticate+authorize adapter may build it. Tests manufacture it under
`#[cfg(test)]` to study transaction behaviour. Therefore this module
**cannot receive an untrusted network packet** yet. It cannot claim the bytes
are genuine W3 ciphertext, and it is not a real end-to-end chat.

The receiver's signing key is supplied externally, but checked against a
previously authorized, epoch-pinned Akula device signing key before signing.
The ciphertext, immutable digest, signed receipt and sender-bound MessageId
are written in a single WAL/FULL SQLite transaction. The signed receipt
becomes externally available *only after commit*, never on failed writes.
Identical repeats return the original receipt, even after a database restart;
altered messages with the same sender+ID are rejected.

No runtime background worker, key store, independent account, central server,
file attachment handler, Roha transport or Bluetooth service is introduced.
The known trust-epoch revocation race across independent modules remains a
blocker until a shared atomic authority fence exists.

Before enabling this lab in the actual Akula app:
- integrate the genuine WOW W3 authenticated envelope;
- implement host-approved durable device enrollment and revocation;
- enforce storage location custody/permissions and queue quotas;
- implement network sending, return receipt path and UI;
- test installed-client delivery with independent devices, then mobility.

Checks: in aquilum-app, run
`cargo test --locked -p aquilum-core --features messaging-w4-inbox-lab messaging::`
and `cargo check --locked -p aquilum-core --no-default-features`.

Notion: https://app.notion.com/p/40032a84beec81919ec8df2e31056543
