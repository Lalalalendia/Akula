# STY-DEVICE-01a — owner-signed device roster verification (inert laboratory)

Builds on Akula UI-01 (merged PR #513) and read-only UI-02 (Draft PR #528).
This is an independent Core contract, not an installed trusted-device registry.

- A VaultId, monotonic owner-signed epoch and at most 32 records are signed with a
  versioned, domain-separated canonical binary Ed25519 claim. Every record binds
  one DeviceId, raw Ed25519 device public key, remote EndpointId public bytes,
  and RecordedOnly / Revoked state.
- Requires a separately pinned owner public key AND a separately witnessed
  monotonic epoch floor. For a candidate at exactly the witnessed epoch,
  requires an independently stored, domain-separated snapshot digest: a different
  *validly signed* snapshot at the same epoch is rejected as WitnessMismatch.
  Rejects signature edits, cross-vault or wrong owner,
  replay below the external floor, nil IDs, zero keys, duplicate IDs, duplicate
  EndpointId keys, nonsorted records and oversize input. No JSON canonicality risk.
- The result is an immutable read-only observation. RecordedOnly is NOT
  IdentityChecked and the verifier never issues File Gate or network grants.
- It runs behind optional Core feature device-roster-lab. No installed binary,
  socket, peer discovery, key signing, or file mutation is activated.

Required next stages: trusted OS/installer-owned first enrollment and independent
durable epoch witness; device private-key custody, interactive invitation and
fresh challenge of the live QUIC peer; signed revoke/rotate with invalidation;
separate VaultId + object-root scoped File Gate grants rechecked after staging
and immediately before user-file mutation; isolated-process then real two-device
Linux/Windows tests.

These are non-negotiable; passing Rust unit tests of a synthetic key pair
does not prove production enrollment, owner intent, or physical device identity.
Network send, APPLIED, durable ACK and File Gate ALLOW remain HOLD.

Acceptance: exact-head cargo test of the feature, Rustfmt and module-scoped strict Clippy
on Linux and Windows. The new module has `#![deny(clippy::all)]`; inherited
Core-wide Clippy style warnings are reported but do not fail this unrelated lab.
Do not auto-merge before those results.
