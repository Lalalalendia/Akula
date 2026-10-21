# STY-TRANSFER-UI-02 — read-only device roster / intended recipient

Parent: [STY-TRANSFER-UI-01](STY-TRANSFER-UI-01.md). UI-01 is merged into Akula main in [PR #513](https://github.com/Lalalalendia/Akula/pull/513).

This patch prepares the device list and an **intended recipient** selector inside the existing Akula transfer dialog. It does not create a trusted peer registry, a pairing protocol, an online detector, a signed Device↔Endpoint binding, a Vault ACL, or a file transfer entitlement.

- Device identity is a typed `aquilum_core::sync::DeviceId` plus a pinned 32-byte public-key fingerprint. UI rows are bounded to 32, sanitize control characters and display only a short public fingerprint.
- Statuses are `Unverified`, `IdentityChecked`, and `Revoked`; unknown/revoked/zero-key identities cannot be selected even in tests. The 'IdentityChecked' label explicitly states that identity does not imply Vault/File Gate rights.
- Selection is an in-memory *(DeviceId, key fingerprint)* tuple only. Replacing the verified snapshot with a revoked entry or a rotated key clears selection. Any duplicate DeviceId (even with the same key), zero-key record or oversized roster must remain unselectable. Click actions carry the exact typed DeviceId and full public-key fingerprint, not an index that can change after rendering.
- **Production source remains unavailable by design**: the only way to inject a nonempty device list at this stage is an explicitly `#[cfg(test)]` fixture. Therefore production still shows 'No verified devices' and never invents peers. Real discovery and owner-vetted registry integration are separate work.
- A Masonry harness test proves a real pointer event selects the unique test-only verified identity, and that revocation clears selection. Tests also falsify unknown/revoked/zero-key/ambiguous/rotated identities.
- There is no 'Send' action. No QUIC, File Gate grant, APPLIED, durable ACK, file read/write or network side effect. A visible intended recipient **is never authorization**.

Next adapter: integrate a real first-party Akula trusted-device source with signed Owner binding to DeviceId/endpoint/complete Vault ACL, independent freshness/revocation checks, and a fresh check immediately before any future File Gate-issued transfer. Two OS processes are useful lab evidence, but do not replace physical two-device acceptance. Until that separate gate: **HOLD**.
