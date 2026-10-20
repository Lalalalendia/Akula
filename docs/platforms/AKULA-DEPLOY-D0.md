# AKULA-DEPLOY-D0: GUI-free Core baseline and server admission boundary

**Status:** developer-only D0 proof. NOT a server, container, Android client, web API, remote writer, device-pairing system or shipping release gate.

Parent plan: [AKULA-DEPLOY-01](https://app.notion.com/p/3fe32a84beec817cb9b7e7fa5a552334). Preserve the existing Desktop/Sync/File Gate gates.

## Verified source inventory

| Concern | Owner in Akula | What the current code means for headless |
| --- | --- | --- |
| Core | [lib.rs](../../aquilum-app/core/src/lib.rs), [Cargo.toml](../../aquilum-app/core/Cargo.toml) | Rust library contains files, history, search, MCP and Sync, independent of the native UI crate. |
| Native UI | [native Cargo.toml](../../aquilum-app/native/Cargo.toml) | Masonry/winit/vello_cpu only belong to the desktop host; no GUI should be linked by a server. |
| Boot | [app_core.rs](../../aquilum-app/core/src/app_core.rs) | Core::open constructs settings, UI state SQLite, history, documents workers, search and watcher. This is NOT a no-side-effect read-only constructor. |
| Startup storage | [settings manager](../../aquilum-app/core/src/settings/manager.rs), [UI state](../../aquilum-app/core/src/ui_state/service.rs) | Missing settings can be written; UI SQLite state can be created. Provide a writable, PRIVATE appdata directory different from the content vault. |
| Snapshot | [file document](../../aquilum-app/core/src/files/document.rs) | read_file_snapshot_impl reads bytes and reports their raw BLAKE3 hash. Only use fixed trusted paths in D0; no remote path ingress. |
| MCP | [MCP host](../../aquilum-app/core/src/mcp/mod.rs), [HTTP loopback server](../../aquilum-app/core/src/mcp/server.rs), [tool router](../../aquilum-app/core/src/mcp/tools/mod.rs) | Loopback with token when enabled. Tool router contains mutating commands, so it MUST NOT be treated as a safe read-only remote server API. |
| MCP path resolution | [Vault](../../aquilum-app/core/src/mcp/vault.rs) | Rejects lexical traversal and symlinks/reparse preflight, but explicitly warns about local TOCTOU races. Need stronger root handles for remote service. |
| Index | [search service](../../aquilum-app/core/src/search/service.rs) | Preparing an index creates persisted state and background workers. Do not assume read-only calls never write private appdata. |
| Write authority | [File Gate](../../aquilum-app/core/src/files/gate/), [host custody registry](../../aquilum-app/core/src/sync/host_custody_registry.rs) | Existing safeguards are feature-, installation- and trust-bound; arbitrary headless writers are NOT admitted merely by constructing Core. |

Current workspace members: aquilum-core and aquilum-native. No packaged Akula headless/server process is present.

## D0 smoke test

Source: [headless_core_smoke.rs](../../aquilum-app/core/tests/headless_core_smoke.rs).

1. Create separate disposable vault/ and appdata/ paths.
2. Start Core using a no-op EventSink without a winit, Masonry or OS window.
3. Verify no MCP server has started and there are no registered workspaces.
4. Read one fixed Markdown note through the existing Core snapshot function; verify raw source text and original BLAKE3.
5. Stop Core; check original bytes and original single-file vault tree unchanged.
6. Restart Core against the same *private* appdata and repeat.

CI: [core-headless-smoke.yml](../../.github/workflows/core-headless-smoke.yml). Dedicated Linux lane compiles/runs Core with NO installation of native GUI development libraries. It also checks the optional host-custody build.

Local commands:

    cd aquilum-app
    cargo test --locked --no-default-features -p aquilum-core --test headless_core_smoke
    cargo test --locked --features host-custody -p aquilum-core --test headless_core_smoke

## Non-claims

- D0 compiles a library and runs an isolated local test, NOT a real standalone daemon binary.
- No HTTP listener, read-only role enforcement, authentication, per-object ACL, pairing, revoke, bounded RPC, rate limits or remote path resolver is added.
- D0 only requires the *vault* remain byte-for-byte unchanged. Core MAY mutate its private appdata (settings and SQLite). The test itself creates its fixture.
- A raw snapshot function does not prevent an untrusted caller from reading arbitrary files or following symlink races. Never expose it directly to a browser, agent or network user.
- No proof yet that full Core with Tantivy, SQLite and workers fits RouterOS memory/flash budgets. No Android/TV, ARM64 or actual OCI build is part of D0.
- This does not alter Core feature defaults, native runtime, File Gate or installed-host authority.

## Next separate gate: D1 read-only Node

1. Standalone minimal daemon with a narrow, statically allowlisted interface (health + list/read permitted content only).
2. Trusted root and persistent identity pinned *before* serving; no arbitrary absolute paths; open file descriptors safely against symlink and rename races.
3. No public WAN bind. Listen only on loopback initially, with authentication before metadata or expensive file processing.
4. Hard limits on request body, response bytes, open files, concurrent work, total read size and time.
5. Negative tests for traversal, symlink/reparse point, root swaps, size exhaustion, wrong token, no token, no permission, late revoke, service restart.
6. Verify the content vault hash tree remains unmodified through all requests; kill/restart must not corrupt private SQLite state.
7. Any future write or multi-device Sync must go through separately verified File Gate host authority and CAS; no shortcut via MCP or raw fs writes.

Release gates remain independent. Do not expand Akula into a monolith; create platform-specific runtime packages only after the Core/API contract is stable.
