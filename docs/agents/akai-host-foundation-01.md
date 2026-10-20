# AKAI-HOST-FOUNDATION-01 — incorporate independently checked Broker source without authority

**Status: candidate source consolidation, not shipped security functionality.**

## Why this milestone

The Akula MCP credential and grant stores in `main` are intentionally fail-closed. The parallel Staya RN-4E1b Broker research has real Windows OS-level checks, but until its source is present in the main repository as a separately testable target, subsequent Host-issued registration would have to duplicate code or rely on fragile stacked research branches.

This change imports the **exact source blobs**, with no changes to runtime security semantics, from the existing [Staya #467](https://github.com/Lalalalendia/Akula/pull/467) head `395933e41`. That commit includes the previously tested #438 → #443 → #447 → #450 → #453 → #460 → #464 chain. The only textual updates are the standalone README, this ADR, and a new independent CI workflow.

## In-scope

- Standalone `aquilum-app/host-broker-lab/` Cargo workspace only, not listed in the Akula workspace.
- Bounded deny-only IPC, Windows LocalSystem service/client SID checks, SCM stop/cleanup, OS-pinned protected storage marker with DACL checks.
- Journal/revoke and external-witness comparison under **`cfg(all(test, windows))`** only; no production issuance, grant or signed root registration.
- Unified Windows/Linux Rust validation plus **real temporary Windows LocalSystem SCM** fixture: missing state fails; protected state starts; Users write causes rejection; test resources removed.
- No executable installation, native/Core changes, plugin grants, MCP routing, P2P sync or File Gate paths.

## Required acceptance

1. Windows and Linux `cargo test --locked --all-targets`, `cargo check`, and `cargo fmt --check` pass with a stable, independent `Cargo.lock`.
2. Real Windows SCM positive-and-negative DACL fixture passes at exact HEAD, including a missing marker and user-write tampering. This checks a disposable unsigned CI service, **not a production signed installer**.
3. Main Application CI (if triggered) does not regress. Changed file list is limited to the standalone subtree, its disposable fixture, this ADR and its dedicated workflow.
4. Permission vocabulary stays bounded to three negative wire statuses; no `Allow` response, executable MCP client or read path.

## Explicitly unresolved

- An actual signed and installed Broker binary/service owned by SYSTEM or a declared trust principal.
- A fixed, immutable local-vault/root enrollment from an independently protected installer/owner, including a separate local replica identifier and `plugins::VaultId`.
- A single broker-owned monotonic grant epoch with protected WAL/SHM (or equivalent single-file store), rollback witness and cold recovery; #467 test-supplied witness is **not** a witness provider.
- A mutually authenticated IPC channel for agent-specific, capability-specific decisions and physical-root binding under locked lifetime; then a real MCP response-time revoke fence and official SDK A/B evidence.

Untrusted SQLite records or self-issued registration receipts MUST NOT be accepted into production `HostPinnedReadRoot`. Staya QUIC File Gate/remote sync is a separate feature: do not couple local agent read readiness to P2P release.

[Master](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de) · [Host research #460](https://github.com/Lalalalendia/Akula/pull/460) · [Journal #464](https://github.com/Lalalalendia/Akula/pull/464) · [Witness contract #467](https://github.com/Lalalalendia/Akula/pull/467)
