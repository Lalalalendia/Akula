# Akula Core inventory — candidate plugin integration points

Source baseline: GitHub `main` commit `939d5109f19a990e69cc2cbfbc1f046d941e9766` (2026-10-17). This records **observed code**, not functionality already available to community plugins. Repository: https://github.com/Lalalalendia/Akula

| Existing subsystem | Actual source | What can be reused | Boundary / work before exposure |
| --- | --- | --- | --- |
| Core composition and events | [app_core.rs](../../aquilum-app/core/src/app_core.rs) | `Core`, `CoreEvent`, `EventSink` | Events can be emitted while locks are held; plugin calls must be async/queued, filtered by grant and must not call synchronously back into Core. |
| Core public module structure | [lib.rs](../../aquilum-app/core/src/lib.rs) | Independent Rust Core without native UI | Do not export raw `Core` internals, paths or service pointers to guest plugins. |
| Editor sessions | [documents/hub.rs](../../aquilum-app/core/src/documents/hub.rs) | `DocumentHub`, versions, dirty-buffer awareness, interlocks | Existing Hub metadata is not a plugin capability; PLUGIN-09 must handle stale sessions and native editor state. |
| Atomic write and history | [files/gate/write.rs](../../aquilum-app/core/src/files/gate/write.rs) | `expected_hash`, atomic save, history recording, index notifications | It accepts ordinary `&Path` with no plugin ACL; put grant/identity/conflict checks **before** calls and recheck at side-effect boundary. |
| Search/index | [search/mod.rs](../../aquilum-app/core/src/search/mod.rs) | Search and links | Search hits, paths, tags and snippets leak data; filter by the same scopes as content reads. |
| Local history | [history/mod.rs](../../aquilum-app/core/src/history/mod.rs) | Version history and recovery | Must not expose private other-vault paths/history without a grant; do not bypass retention safety. |
| MCP server | [mcp/mod.rs](../../aquilum-app/core/src/mcp/mod.rs) | Agent-facing note/search operations | Existing HTTP/STDIO token is **not** a plugin authorization token; do not use MCP to bypass broker. |
| Built-in sync | [sync/mod.rs](../../aquilum-app/core/src/sync/mod.rs) | Typed provider/VaultPort contracts and safe sync plans | Existing SYNC PR 00–11 is a separate, **built-in** workstream. Never promise a community plugin for that first release. |
| Native UI | [native/Cargo.toml](../../aquilum-app/native/Cargo.toml) | Masonry/winit native rendering, command surfaces | No WebView; PLUGIN-10 must render declarative plugin UI, not arbitrary host-owned widget pointers. |
| PR gate | [app-checks.yml](../../.github/workflows/app-checks.yml) | Stable `Main PR gate` and targeted Rust tests | Reuse it; no extra required status check, no always-on heavy benchmarks. |

## Proposed ownership (not existing code)

- `aquilum-app/core/src/plugins/` — versioned plugin API, grants and scoped façade into Core; do not fork search/history/file logic.
- `aquilum-app/native/src/app/` — plugin management UI, consent dialogs and declarative views.
- `docs/plugins/` — ADRs, contracts, fixtures and the evidence matrix.
- Optional isolated runtime crate/process after PLUGIN-05's measured comparison; **not a PLUGIN-00 dependency**.

## Current non-goals

- No community plugin loading in current `main`. No marketplace, permission grants, sideloading or WASM/JS runtime.
- Sync is not a dynamically loaded plugin; it remains a built-in Rust Core module with an independent roadmap.
- No modification of the [canonical built-in sync roadmap](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11).
- No change in native editing semantics or branch protection.
- No promise that existing Obsidian JS plugin packages will run in Akula.
