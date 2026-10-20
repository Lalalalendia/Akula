# AKAI-AUTH-01b0 — durable agent/vault read-grant foundation

**Status:** inert implementation in Draft PR. **No production MCP read/write authority.**

## Dependencies and rationale

- [#407](https://github.com/Lalalalendia/Akula/pull/407) provides distinct host-issued MCP principals and a durable, revocable credential registry.
- [#418](https://github.com/Lalalalendia/Akula/pull/418) is a separate, deny-all scoped MCP HTTP mode. Its production-facing dispatch remains disabled even if this PR is merged.
- The canonical broker [`plugins::Capability`, `GrantScope`, `VaultId`](/Lalalalendia/Akula/tree/main/aquilum-app/core/src/plugins) names the permission vocabulary. Do **not** register an MCP agent as a fake plugin.
- Stable `sync::VaultId`, `plugins::VaultId`, a local workspace pathname, and `HostVaultRegistration` are **different concepts**. A trusted host must independently establish a mapping from its installed/pinned physical vault registration to the permission registry. Coincidentally equal UUIDs are not authority.

## Stage implementation

The inert `mcp::agent_vault_grants` module provides:

1. `HostVaultRegistration` for a host-supplied stable `plugins::VaultId` plus a 32-byte root-binding digest. **Its constructor checks format only**, not trust provenance. It must not be created from MCP arguments, an untrusted path, `clientInfo`, or an untrusted sync peer's `VaultId`.
2. Dedicated SQLite v1 under host app-data, WAL/FULL/foreign keys, explicit `provision_new` and `open_existing`. Ordinary reads never auto-create/repair a missing registry; unexpected version/tables/integrity fail closed. Installer ownership and Windows ACLs, trusted sidecar custody and rollback defense are still required.
3. Stable host vault binding: a `VaultId` can be registered once; presenting a changed physical-binding digest with the same ID is refused. Mere registration creates **zero grants**.
4. Canonical host-approved permission: only `Capability::NotesRead` + `GrantScope::WholeVault` are accepted. No write, search, cross-vault move, destructive or UI authority.
5. Grant/revoke under `BEGIN IMMEDIATE` with expected-epoch CAS, monotonic policy version, optional expiry and audit. A revoked or expired grant never creates a fresh read lease. Reopening the existing SQLite database preserves the decision.
6. `AgentReadLease` and revalidation enforce store epoch/expiry and vault binding **within this isolated store only**. They **do not** check the current agent credential epoch, physical file handle, native host custody or File Gate commit boundary. Thus the lease must **never** authorize live bytes until these checks exist.

## Negative tests

- Agent A → registered vault X permitted; A → Y denied.
- Agent B → Y permitted; B → X denied.
- A deliberately forged root-binding digest for X is rejected even when UUID matches.
- Grant without prior registered host vault is rejected.
- Stale expected epoch cannot overwrite a newer grant or revoked decision.
- Revocation survives close/reopen and invalidates existing read leases.
- Expiry blocks access.
- Missing/unsupported or corrupt SQLite state fails closed; no silent empty-ACL fallback.
- Native Windows and Linux Rust workspace test lanes must pass on exact branch HEAD.

These tests validate **the isolated policy storage model**, not two real opened vaults through the official MCP SDK.

## To actually enable read_note (AKAI-AUTH-01b1)

1. Add a trusted host-only binder that reads independently protected host custody enrollment, pins/verifies the actual root and maps one stable root to exactly one canonical vault ID. Prove a rename/reparse/symlink/path-alias cannot select another root.
2. Present host-approved grants through a permission-management UI/API that is **not itself exposed to MCP**. Verify credentials are currently live and reconcile authorization epochs across credential store and grant store.
3. Resolve `workspace` using the registered host index; do not treat the string as an identity. Resolve every `notes[]` entry before reading, revalidate grants before byte disclosure, and return a generic denied response without titles/paths/existence hints.
4. Allow scoped `read_note` only with an `AuthorizedToolContext` that carries a verified principal, registered host vault and current lease. Other 29 tools remain denied; `tools/list` must reflect the allowed subset.
5. End-to-end on the **official MCP SDK**, both Linux and Windows: separate A/B Bearers, registered vaults X/Y, A→X read success, A→Y denied, B→Y success, B→X denied, revoked and expired denied, legacy Bearer denied, batch path alias attempts denied, no leaked metadata.

Do **not** interpret acceptance of this PR as acceptance of 01b1/01c search filtering or 01d write/commit fencing.
