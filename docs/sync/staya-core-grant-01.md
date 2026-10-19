# STY-G3 / RN-4C — Core-registered P2P grant epochs (disabled research)

**Status:** experimental Core journal; [stacked on signed grant gate PR #391](https://github.com/Lalalalendia/Akula/pull/391). No enrollment UI, product sync, background workers, File Gate mutation rights or release merge.

## Why this module belongs in Core

The lower-layer QUIC adapter [PR #379](https://github.com/Lalalalendia/Akula/pull/379) authenticates a network EndpointId, but cannot decide whether a particular DeviceId has permission to fetch an object from a particular VaultId. Signed-scope [PR #391](https://github.com/Lalalalendia/Akula/pull/391) rejects malformed, expired, revoked and cross-destination fixture grants, but its trusted policy is constructed in memory and has no persisted epoch.

Akula already owns typed `sync::{DeviceId, VaultId}`, registered workspace physical-root preflight, file/history custody and SQLite journals. A laboratory **read-only authorization snapshot** therefore belongs in an *explicit opt-in Core module*, not the Roha/noq transport and not a second File Gate.

## Scope of the code

- New `aquilum-app/core/src/sync/peer_grant_lab.rs`, guarded by default-OFF Cargo feature `staya-core-grant-lab`. The native app does **not** enable it.
- Opens only a pre-registered Core workspace via the existing `RegisteredReadAdmission`; stores workspace UUID and observed device/inode identity. This workspace ID is a **machine-local registration key, not a shared VaultId**.
- Explicit `provision_new` to an absolute appdata SQLite path **outside any registered workspace**. Normal `open_existing` is fail-closed for missing/unexpected schema/root and never initializes an empty grant database.
- Versioned SQLite schema, WAL, `synchronous=FULL`, `BEGIN IMMEDIATE` for issuance and revocation. Single active pull per (VaultId,DeviceId) lab tuple, exact EndpointId, issuer verification key, BLAKE3/size, expiration, nonce. No private key storage.
- A duplicated active issuance is refused. Revocation writes an epoch+1 tombstone; the same grant can only be reissued under a later epoch. `revoke(expected_epoch)` uses compare-and-set so stale callers cannot erase a newer grant.
- `read_active` rechecks registered physical root, grant state and expiry, returning only a read-only snapshot. It is **not** an atomic revocation/write lease and cannot call File Gate.
- Validation tests check missing/corrupted/unregistered stores, cold restart, cross-vault and wrong physical workspace, expiry/oversize, idempotent fail-closed revoke, monotonic reissue, concurrent/stale epoch and **zero user-vault writes**.

## Explicit trust limits

**This is not a production trusted issuer.** Trusted host code must explicitly call `issue` with a real user-approved shared VaultId and signed grant metadata. This research does not validate who requested issuance. It cannot produce a signed grant, cannot authenticate the installed host OS/broker, cannot shield its SQLite DB against an uncooperative same-user actor and cannot guarantee power-cut persistence. Host `RegisteredReadAdmission` protects registered root reads; it is **not** File Gate write authority.

The stored local workspace UUID must **never** be converted into a canonical shared VaultId. `read_active` snapshots may become stale before use; an actual file mutation must be fenced with **current Core policy + pinned registered root + File Gate publication lease**, and verified staged/Bao bytes. Nothing can be marked `APPLIED` or gain a durable replica receipt merely because the grant database says active.

This new module is intentionally separate from `peer_scope.rs` and from actual network bytes until tests prove safe binding.

## Acceptance and next milestones

1. Final HEAD CI: `cargo test --locked -p aquilum-core --features staya-core-grant-lab --test rn4c_peer_grant_lab` on Linux/Windows; rustfmt; no changed Cargo.lock. CI pending.
2. Bind the verified signed-scope proposal to the **fresh** `StoredPull` with typed IDs; reject epoch/expiry mismatch and cross-process replay.
3. Establish an atomic or fail-closed replay fence across revocation and File Gate, not a stale read-before-write snapshot.
4. Get actual authorized Roha QUIC bytes into Bao-authenticated staging, then Core File Gate no-overwrite/readback/durable receipt; no `APPLIED` earlier.
5. Real Windows-PC ↔ Android phone, crash/SIGKILL/restart, backup/rollback and UI enrollment acceptance before product activation.

Source: [STY-MASTER-PLAN-02](https://app.notion.com/p/3fe32a84beec81e5acf7c081fe5e0bfd). The [existing File Gate receipt PR #335](https://github.com/Lalalalendia/Akula/pull/335) is not modified.
