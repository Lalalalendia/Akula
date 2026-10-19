# RN-4D / APP-GRANT-FENCE-01 — revocation-to-preflight race

**Status:** opt-in research only, stacked on [Core grant epochs #398](https://github.com/Lalalalendia/Akula/pull/398). Never enabled in native Akula. No actual file publication or P2P sync.

## Why the previous stage was insufficient

[RN-4C](https://github.com/Lalalalendia/Akula/pull/398) can persist revocations but exposes `read_active`, a snapshot. After reading the snapshot, another Core could revoke the device before an operation using the cached permission. Reading the same epoch *again* without holding a cross-connection fence still leaves a check/use gap.

This experiment adds `RegisteredPullGrantLedger::with_fenced_pull_preflight` behind the **existing default-off** `staya-core-grant-lab` feature.

1. Revalidate the existing registered workspace and physical root.
2. Acquire a **SQLite BEGIN IMMEDIATE transaction**, which prevents other SQLite writer transactions (including `issue`/`revoke` from another Core/process) from committing until the fence completes.
3. Re-read the full active grant *inside* this transaction: exact VaultId, typed DeviceId, EndpointId, issuer public key, object digest + length, stored nonce, epoch and expiry. Refuse revoked/expired, changed scope or mismatched actual peer identity.
4. Run a **bounded read-only research probe** while the SQLite writer reservation remains held; recheck the registered root and close/commit the transaction. No grant/handle to the File Gate is returned.
5. If a revocation commits **first**, the probe is denied. If this fence takes the SQLite writer reservation first, a later revocation waits or fails SQLITE_BUSY. This is a cooperative SQLite *ordering* guarantee, **not an absolute promise that a clicked revoke interrupts an already-admitted operation**.

The existing `read_active` and fenced path now share one row decoder to avoid different validation rules.

## Concrete negative tests

- An independent SQLite connection with zero busy timeout attempts a revocation update during a live guarded read. It must fail SQLITE_BUSY, then revoke successfully after release.
- After revocation and a new issuance, old epoch preflight must not enter the probe.
- A different authenticated QUIC EndpointId, another vault, a changed issuer/nonce/object length/digest or expired grant is denied **before** the probe callback.
- An unsuccessful probe releases the reservation (rollback), allowing subsequent revocation; no held lock is leaked.
- Prior registered-root, cold-start, expiry and physical-identity RN-4C tests stay green; no user workspace file or receipt is ever created.

## Critical limits and design consequence

This does **not** authorize files or solve all TOCTOU. The callback is only a lab probe: do not call a File Gate writer from it or enable the feature in native Akula. The actual authenticated QUIC `remote_id()` must be supplied by trusted caller-side connection state; tests use a constant. No wire-signed grant or local user enrollment is checked by the Core code. A caller-supplied clock is not trustworthy without a host clock policy. This SQLite database is appdata and has no independent OS installer/broker trust; hostile same-user actors could manipulate/replace it outside normal cooperative SQLite transactions.

For eventual production, the **short** publication check must occur **after** bounded authenticated Bao download and staging, while holding independently pinned OS path/File Gate custody. Holding a DB writer lock through a long download would block revocation and is forbidden by design. Even a successful preflight never means `APPLIED` or durable replica receipt: first complete File Gate publication under the real write authority, then readback, durable journal and a host-approved receipt. Crash/power-cut/Windows and real Android↔PC remain separate acceptance gates.

## Evidence

Dedicated [RN-4D Linux/Windows CI](https://github.com/Lalalalendia/Akula/blob/experiment/staya-rn4d-revocation-fence-20261020/.github/workflows/staya-rn4d-revocation-fence.yml) checks this code and the RN-4C suite under exact `cargo test --locked`. Keep Draft pending success.

[STY-MASTER-PLAN-02](https://app.notion.com/p/3fe32a84beec81e5acf7c081fe5e0bfd)
