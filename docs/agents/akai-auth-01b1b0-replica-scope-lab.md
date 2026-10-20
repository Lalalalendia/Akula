# AKAI-AUTH-01b1b0 — falsifier: cooperative host records are not trusted authority

**Scope:** read-only test in the existing Core Host Custody registry. No new runtime validator, MCP route, grant, credential, installer, transport, build feature, workflow or File Gate behavior.

## Why this proof is needed

A structurally correct `SharedHostCustodyRegistry` entry can match a
physical root, remote Sync VaultId, profile and external lock without proving
**who issued the entry**. A malicious process with same-user privileges may
provision a second valid SQLite host ledger and replay the same record. The
record hash and consistent query result do not authenticate installer ownership.

`sync::VaultId` is the identity of a shared remote vault. Two independent
local replicas may legitimately share that value; permission checks must bind
a specific local physical replica. Do not equate it automatically with
`plugins::VaultId` or a workspace UUID.

## Implemented test

`sync::host_custody_registry::tests::copied_valid_host_ledger_is_not_installer_authorization`:

1. Provision one valid, private host-ledger fixture and enroll a root.
2. Provision a second valid ledger at a **different same-user path** and enroll
   the **same** record, including physical root ID, remote VaultId, profile and
   lock identity.
3. Prove `inspect()` returns the **same positive result** on both.
4. Cold-reopen the second ledger and verify it still returns the record.

This is a deliberate positive counterexample proving that cooperative
registration cannot be elevated to a trusted bearer, host custody receipt or
MCP grant. The test does not create or allow any of those things.

## Shared implementation owner

- [Staya PR #427](https://github.com/Lalalalendia/Akula/pull/427):
  canonical Core `inspect_exact_replica_scope` candidate for the registered
  local root/profile/lock and shared remote VaultId. Its acceptance is
  independent; do not introduce an equivalent MCP runtime implementation.
- [AKAI PR #432](https://github.com/Lalalalendia/Akula/pull/432):
  merged test-only pinned read candidate, no production constructor.
- [Staya PR #434](https://github.com/Lalalalendia/Akula/pull/434):
  Windows OWNER/DACL negative checks, not a positive installation authority.
- [Staya PR #438](https://github.com/Lalalalendia/Akula/pull/438):
  isolated deny-only LocalSystem pipe protocol, not an installed issuer.

Before activating `read_note`, an independently OS-protected installed host
Broker must attest the local replica and issue the canonical local permission
`plugins::VaultId` mapping, with policy/credential revalidation at the
network disclosure boundary and Windows/Linux real two-agent SDK E2E.

## Acceptance

`cargo test --locked -p aquilum-core --no-default-features --features host-custody --lib sync::host_custody_registry::tests::copied_valid_host_ledger_is_not_installer_authorization -- --nocapture --test-threads=1`

The existing Linux and native Windows Host Custody matrix must pass on
**exact PR HEAD**, plus the Rust workspace, main gate and unchanged legacy MCP
behavior. A PASS proves only that the same-user fake is observable and
therefore **must not be trusted**. It is not a production authorization proof.

Master plan: [AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de).
