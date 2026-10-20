# AKAI-AUTH-01e0 — prevent self-issued vault claims from becoming MCP authority

**Status:** Draft gate / test-only proof. Depends on accepted AUTH-01d0 #458 and AUTH-01d1 #469. No live read tool, trusted Host Broker, or installer enrollment is introduced.

## Problem

`HostVaultRegistration::from_host_custody` historically accepted any nonzero 32-byte digest. It checked **format**, not protected issuer provenance. A future MCP adapter must not infer that a client-owned SQLite registration or matching local `VaultId` is host authority.

## Exact changes

1. Compile the self-issued `HostVaultRegistration::from_host_custody` factory **only under `cfg(test)`**. In normal production Core there is no public or crate-visible constructor for that receipt. An installed Broker adapter must provide independently verified issuer provenance before any production minting path is added.
2. Add a real loopback MCP server test with two principals, two vaults, two private sentinel Markdown files and valid persisted `NotesRead` grant entries created from test-only untrusted receipts.
3. Demonstrate that grant storage itself can validate both leases, but `start_scoped_deny_all` still exposes an empty `tools/list` and denies single and batched `read_note` from either agent against either claimed vault. Responses contain no file contents, vault names or a fallback to the legacy full-access caller.

## Limits

- Test fixture receipts are intentionally self-issued and **not** protected authority. The test proves denial on today's scoped HTTP runtime, not tamper-proof grant storage or an integrated authorization protocol.
- No code path joins a broker, OS service, physical root, File Gate or `HostPinnedReadRoot`. No real agent reads become available.
- SQLite/WAL/SHM path substitution, same-user adversaries, credential issuer custody, trusted installer-owned root mapping, response-time revocation, network delivery, and official MCP SDK A/B remain unresolved.
- A green test is not evidence of production two-agent read access. Existing legacy MCP behavior is unchanged and must not be treated as scoped access.

## Acceptance

- Exact-head Windows MCP `mcp::server::tests::scoped_http_does_not_promote_user_minted_vault_grants_into_read_access` PASS.
- Linux Rust workspace and required policy/Main PR gates PASS.
- Only the grant factory build visibility, one real MCP regression test and this ADR differ from `main`; no changes to grants, tool routing or File Gate.
- Production library build has no self-issued receipt constructor, by Rust conditional compilation; do not reinstate one until a verified Broker issuer API is accepted.

## Dependency handoff

Broker ownership remains with existing Staya RN-4E1b branch stack; do not create a competing MCP custody validator. Next gate: a single authenticated OS-protected host enrollment and local-replica `plugins::VaultId` mapping. Then require true MCP read response emission/revoke fencing and official two-agent E2E before advertising `read_note`.

[AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de)
