# Akula Host Broker — isolated Windows service foundation

**Source-only, not installed or connected to Akula.** This crate is deliberately an independent Cargo workspace. The desktop application, MCP server, File Gate, package installers and P2P transport do not depend on it.

## Present and tested in an ephemeral Windows service fixture

- A bounded 32-byte named-pipe protocol with only `HOLD`, `MALFORMED`, and `UNSUPPORTED`. No allow or enrollment opcode.
- LocalSystem process verification, restricted named-pipe ACL, client impersonation and per-connection identity checks; SCM lifecycle and cancellable stop.
- Read-only inspection of a **pre-existing** `C:\ProgramData\AkulaHostBrokerResearch\authority.marker` and its pinned parent handles, actual owner/DACL, installer-like service SID and no ordinary-user namespace write. No marker provisioning or repair on the read path.
- A test-only CAS DENY journal and comparison against a **test-supplied** external epoch witness. Those modules are excluded from the normal executable.

## Build and test

```sh
cargo test --locked --manifest-path aquilum-app/host-broker-lab/Cargo.toml --all-targets
cargo fmt --manifest-path aquilum-app/host-broker-lab/Cargo.toml --all -- --check
```

The [dedicated Foundation CI](../../.github/workflows/akai-hostbroker-foundation.yml) additionally runs a disposable Windows LocalSystem SCM + protected marker test, including rejection after granting ordinary Users write access.

## Explicit limits

A successful test is **not a trusted vault registration**. There is no shipped installer, signed binary/service policy, OS-protected credential/grant issuance, physical root enrollment, persistent authorized `plugins::VaultId` mapping, production rollback witness, MCP broker client, QUIC identity verification, or File Gate permission. This isolated code never opens notes for agents.

The next production step is an independently installed issuer with a protected vault-root registry and a narrow authenticated client interface. Never treat a local SQLite receipt, matching UUID or this marker as permission.

[AKAI-HOST-FOUNDATION-01](../../docs/agents/akai-host-foundation-01.md) · [Agent master plan](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de).
