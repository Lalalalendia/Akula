# PLUGIN-03 — SQLite permission grants, revocation and audit

**Status:** implementation in PR, not yet a running plugin system. [Roadmap](https://app.notion.com/p/3fc32a84beec8132bd9cd127ba517080), [ADR-0001](adr-0001-trust-boundaries.md), [broker v0](permission-broker-v0.md).

## Goal and dependencies

Add a durable permission decision layer to the already merged PLUGIN-02 broker using **existing** Rust `rusqlite` / `uuid` dependencies. The SQLite file is **separate app-data**, never imported through vault sync.

## Implemented host-only API

- `PermissionBroker::open_persistent(path)`: own dedicated v1 SQLite grant database; unknown schema/corrupt row **fails closed**.
- `VaultId::from_uuid_for_host / as_uuid`: restore a stable vault ID from trusted host application metadata; no guest authority.
- `grant_host_approved_persistent(handle,capability,scope,expires_at)`: does not request consent; the host must already have checked consent and verified the publisher/package. Writes an atomic SQLite transaction before updating in-memory grants.
- Existing `grant_host_approved` remains **session-only**; no database record, no restart restoration.
- Existing `authorize` and `revalidate_lease` compare persisted epochs on **every protected call**, so cross-instance revocation does not silently leave a stale broker authorized.
- Existing `revoke_all_for_host` transactionally deletes persisted grants and increases epoch with a timestamped audit event. Previously granted leases become invalid.

## Persistence identity and scopes

A grant key is the precise tuple **vault UUID + plugin ID + publisher namespace + artifact digest + capability + scope kind/value**. Pinned install-package digest **must be verified by the future trusted installer**; the manifest's publisher claim is not a cryptographic identity proof. For now we explicitly trust the host caller of registration.

Allowed scopes: Action, CurrentNote, SelectedNotes (up to 512 stable note IDs), Folder (lexical only), WholeVault, NetworkHost (exact declared hostname). Read/search/write do not inherit each other's grants. Network grants still cannot open sockets — network/SSRF enforcement is PLUGIN-15, OS path confinement is PLUGIN-04.

An optional expiry in Unix seconds is checked on every authorization; expired grants remain in the DB as inert historical entries until a maintenance operation. A record that cannot be decoded or violates this package's requested capabilities **fails registration**.

## Crash and concurrent-process safety

Writes use SQLite WAL, `synchronous=FULL`, and `BEGIN` transactions; revocation deletes grant rows, changes durable epoch and appends the audit event in one transaction. A different broker process with stale cached permissions receives `StaleLease` (fail-closed), and must unload/re-register against the new epoch. On DB errors, the broker does not optimistically authorize an uncommitted grant.

Audit events store only event kind, vault, plugin/publisher/digest, capability kind and scope **kind**. **No raw folder path, document name, snippet or credentials** in audit; the encrypted private vault is unaffected.

## Known exclusions / later stages

- No consent UI, package signatures, guest loader, host RPC, SDK, real vault files, cloud/network requests or OS sandbox.
- **Exactly-once interactive grants are not implemented here:** short-lived session access may be used with explicit revocation, but durable exactly-once side effects require the transactional host-operation layer (PLUGIN-09/11). Do **not** advertise exactly-once until it is actually proven.
- Caller must use a stable vault ID from trusted metadata across restart, and must store this database **outside** the vault.
- Cross-process epoch checks are conservative and may reject until reload. Future performance work may replace per-call SQLite reads with a proven cross-process revocation cache without changing semantics.

## Acceptance and product value

Unit cases cover exact-identity restart restoration, session-only disappearance, restart-safe revoke, cross-process epoch invalidation, selected-note isolation, invalid store schema and corrupt scope, DB write rollback, expiry, audit minimization and missing-store denial. No new dependencies or required GitHub Actions checks.

Existing `Main PR gate` runs `cargo test --workspace --locked` and `cargo test -p aquilum-core --features sync-engine --locked` on exact PR HEAD. **Product gain:** durable, auditable scoped access that works locally/offline without changing the existing Markdown/Core/Sync behavior.
