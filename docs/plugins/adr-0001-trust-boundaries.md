# ADR-0001 — Trust boundaries for Akula plugins

Status: **Proposed (PLUGIN-00)**. Recorded 2026-10-17. Revisit only by a new ADR with explicit threat-model and regression-test updates.

## Context

Akula is a native Rust Markdown knowledge base. Its current `Core` composes documents, search, local history, settings, workspace watcher, Wikixiv and MCP; the UI is Masonry/winit without a WebView. `CoreEvent` and existing file methods are **internal** interfaces and are **not** ready-made safe public plugin APIs. Local Markdown vaults may include untrusted content imported from the internet or synchronized between machines.

We want a broad third-party extension ecosystem without granting an extension arbitrary filesystem, network, clipboard, shell, native module or OS credential access.

## Decision: host owns authority, plugin owns only data

- **Host/Core (trusted):** owns filesystem access, vault handles, document sessions, file history, search index, service credentials, the native UI and permission decisions.
- **Guest (untrusted):** runs in an isolated runtime selected at PLUGIN-05; no ambient filesystem/network/process/env/FFI access. A plugin issues only typed host calls. An ordinary plugin never receives `Arc<Core>` or unrestricted Rust/OS pointers.
- **Package/registry (untrusted input):** manifests, publisher metadata, compressed assets, update records and plugin-provided UI declarations require validation before any side effects. Publisher identity and signature attest provenance, not benign intent.
- **Vault (untrusted content):** files, Markdown, attachments, links and synced data are **content**, never installation instructions or auto-executed scripts. Grants and installed executable artifacts live outside vault synchronization.
- **External providers (untrusted remote):** HTTP endpoints and OAuth providers are mediated by a network/credential broker after opt-in; render-time external images/resources are a separate network escape path.

## Non-negotiable invariants

1. **Deny by default.** A package may *request* rights, but only a host-owned grant can authorize a scoped action.
2. **Identity bound to installed instance.** The host binds each request to publisher/plugin/bundle digest, vault, action, resource, consent and revocation state; it ignores guest-declared identities.
3. **No permissions inherited between vaults.** A current-note grant is not permission to search every file or read full paths/tags/graph metadata.
4. **No direct guest filesystem API.** Resolve permissions and real path identity together, using capability-oriented file handles; mitigate Windows junction/UNC/casefold, symlinks and races. Never turn a capability-bound lookup into a later unrestricted path reopen.
5. **No direct write.** Authorized writes must coordinate with `DocumentHub`, respect dirty buffers and expected revisions, then use `File Gate` to preserve history and reindex. Move/trash/delete get stricter and inspectable scopes.
6. **All outward channels are guarded.** Network calls, remote Markdown/image previews, MCP/localhost/IPC bridges, external links, clipboard, user-selected files and optional LLM integrations cannot bypass host policy.
7. **Bounded execution.** Guest CPU/fuel, memory, host-call time/size/count, queue depth and background task triggers must have measured limits. Guest failure must not stop the editor.
8. **Revocable grants.** A revoke invalidates handles and blocks pending side effects at commit; updating a package does not silently expand its permissions. A revoke cannot retract information already transmitted.
9. **Local-first preservation.** Plugins disabled/uninstalled/crashed must not break Markdown access, existing history, or independent sync; no mandatory cloud store or marketplace.
10. **Separate provenance from sandbox.** Static review, signatures and malware scans do not enforce runtime resource access. Side-loaded packages must obey the same runtime policy as catalog packages.

## Architecture handoff

`UI Plugin Manager / package loader` -> `verified instance identity` -> `isolated runtime` -> `typed Host API` -> `Permission Broker` -> `Core services`. Host-wrapped handles provide access to vault documents and indexed results; no direct delegation of raw OS resources. Any new ingress/egress endpoint must be reviewed for confused-deputy paths.

In PLUGIN-00 the runtime and broker are **not implemented**. PLUGIN-01 will define the versioned contract. PLUGIN-02/03 will implement authorization/revocation independently of runtime. PLUGIN-04/05 will validate file confinement/runtime with real negative tests. PLUGIN-09 guards writes. PLUGIN-10/15/16 guard UI/network/MCP side channels.

## Decisions deliberately deferred

- Wasmtime Component Model + WIT vs Extism vs alternative runtime: decide **after** comparative cross-platform PoC and actual resource/latency measurements at PLUGIN-05.
- Rust guest SDK first; JS/TS toolchain is an independent PLUGIN-14 experiment and does not imply Obsidian binary/plugin compatibility.
- Typed Rust ACL vs Cedar, delegated Biscuit tokens, cap-std integration, TUF metadata and Sigstore provenance: adopt only if concrete requirements justify them.
- Separate OS-isolated worker (Windows AppContainer/macOS XPC/Linux seccomp/namespaces): defense-in-depth decision at PLUGIN-20 after actual isolation and crash tests.
- SDK, package format and external commercial plugin licensing require a separate AGPL-3.0-only compatibility review before public release.

## Tests and falsifiers

See [Threat model T01–T16](threat-model.md) and [machine inventory](architecture-inventory.json). Denial must be tested **through the actual host API** when implemented. This ADR is a trust model, not a security certification.

Reference precedents: [WASI security](https://wasi.dev/security), [Zed extensions](https://zed.dev/docs/extensions/developing-extensions), [Joplin multiprocess](https://joplinapp.org/help/dev/spec/plugins/), [OWASP SSRF](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html).
