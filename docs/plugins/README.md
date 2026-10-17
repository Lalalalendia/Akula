# Akula Plugin Platform — PLUGIN-00 architecture baseline

Status: **design only**. This directory is the architecture source of truth for the first stage of the plugin program. It **does not** enable third-party plugins, grant permissions, or instantiate a guest runtime.

- Program: [Notion PLUGIN-00–22](https://app.notion.com/p/3fc32a84beec8132bd9cd127ba517080) and [Tela PLUGIN-00–22](https://telawiki.com/spaces/456/pages/7489/akula-plugin-00-22).
- Program stage: **PLUGIN-00 — inventory, invariants, threat model and baseline**.
- Current implementation: [ADR-0001](adr-0001-trust-boundaries.md), [Core inventory](core-inventory.md), [threat model](threat-model.md), [baseline](baseline.md), [machine-readable architecture inventory](architecture-inventory.json).
- The machine-readable inventory is validated by [plugin-architecture.test.cjs](../../.github/scripts/plugin-architecture.test.cjs) in the existing Main PR gate. This test checks **documentation integrity and referenced paths**, NOT sandbox safety.
- Related canonical work: [built-in cloud sync plan (SYNC PR 00–11)](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11). Sync is a built-in Core subsystem and has independent safety gates; it is **not** a dynamically loaded plugin.

## Next engineering stages

- **PLUGIN-01:** define typed, versioned plugin ABI, manifest and compatibility fixtures; no runtime.
- **PLUGIN-02:** implement a small typed, deny-by-default permission broker.
- **PLUGIN-03:** per-vault grants, persistence, revocation and instance identity.
- **PLUGIN-04+:** OS-backed path confinement, runtime comparison, read-only host API, guarded writes, native UI, SDK, packages and eventual registry.

## Hard rules

1. Keep `aquilum-core` and native editor usable without plugins or network.
2. Do not expose `Core`, `DocumentHub`, `File Gate`, `McpServer`, raw `PathBuf` or OS handles directly to untrusted guest code.
3. Requested permissions in a manifest are **not** authority. The trusted host authenticates instance identity and verifies an actual grant on each protected action.
4. Do not load arbitrary native dylibs, Node.js or an unrestricted JS runtime as the safe community-plugin execution path.
5. Do not add Wasmtime, Extism, Cedar, Biscuit, cap-std, TUF and other optional candidates preemptively: measure, choose and justify the minimum necessary dependencies.
6. Text synchronization must never imply executable plugin installation, startup or permission synchronization.
7. Every stage requires reproducible tests and a reviewed change to the relevant design boundary.

## Roadmap / ownership convention

The proposed ownership is `aquilum-app/core/src/plugins/` for typed Core-facing host contracts, `aquilum-app/native/src/` for user-facing UI integrations, and (only if justified after runtime PoC) a separate guest runtime crate/process. These paths do **not** exist yet; they are not deployed capabilities.
