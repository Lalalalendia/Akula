# AKAI-CONF-03 — official MCP SDK direct-to-Core integration

**Status:** Experimental; exact-head CI pending.  
**Canonical program:** [AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de), M0/P0.  
**Prerequisites:** [#377](https://github.com/Lalalalendia/Akula/pull/377) version negotiation, [#368](https://github.com/Lalalalendia/Akula/pull/368) revision CAS, [#380](https://github.com/Lalalalendia/Akula/pull/380) official conformance harness; [#384](https://github.com/Lalalalendia/Akula/pull/384) full pass/fail triage.

## Why

The official generic upstream conformance runner reported 5 successful checks in 26 active scenarios for MCP `2025-06-18`, but two apparently successful tool scenarios returned **`isError=true` / unknown tool**. A real integration test must require successful `read_note` and genuinely committed `update_note`, rather than treating any nonempty text response as success. Existing official conformance tests used a temporary credential-injecting HTTP proxy, so they cannot validate direct Authorization/Origin semantics.

## Implementation and isolation

- Reuse the **existing ignored Rust fixture** from #380. Its fresh temporary Core/appdata and vault now create `Conformance.md` via the production **File Gate**, not an untracked filesystem write, and optionally write the canonical fixture workspace path to a file.
- CI uses the exact npm **`@modelcontextprotocol/sdk@1.27.1`** legacy official TypeScript SDK, which the upstream conformance runner also depends on. Installed only in the temporary diagnostic folder (`--no-save --ignore-scripts`); no production npm or Rust runtime dependencies.
- Two official `Client` + `StreamableHTTPClientTransport` instances connect **directly** to `http://127.0.0.1:<ephemeral>/mcp`, passing the disposable bearer as the native `requestInit.headers.Authorization` option. There is **no intermediary proxy, forwarded credential or global test bypass**.
- Assert negotiated protocol **`2025-06-18`**, serverInfo identity, `tools/list` exposing both `read_note` and `update_note`, and an actual `read_note(includeRevision=true)` success: known Markdown bytes and a 64-character raw BLAKE3 revision.
- Send four denied direct HTTP requests targeting a write: absent bearer → `401`, wrong bearer → `401`, disallowed Origin → `403`, explicit unsupported MCP version → `400`. Re-read revision and content to prove denials had no write side effects.
- Have two official clients send simultaneous `update_note(expectedRevision=<same hash>)` edits through File Gate. Demand **exactly one committed write**, one structured `MCP_REVISION_CONFLICT`, and a subsequent read confirming the winner's revision/content with no unauthorized overwrite.
- Keep the fixture on loopback, in one isolated local process. Tokens and paths are not printed in test summary. Reuse the existing Application CI runner, trigger only for PR title `AKAI-CONF-03`, and archive redacted test outputs for audit.

## Acceptance criteria and exclusions

**Pass only if** all official SDK operations and negative assertions succeed; no retry of stale writes, no unscoped vaults, no non-test user data, both client transports close and fixture stops cleanly. Native Rust, sync-engine, frame fidelity and Windows socket matrix must remain green at the exact PR HEAD.

**Not proven:** per-agent identity or separate grants (both official clients use the same test-only Bearer); any MCP `2026-07-28` functionality; full lifecycle/session enforcement; HTTP GET/SSE/chunk semantics; hostile Host/DNS rebinding outside Origin checks; independent installed binaries on Windows/macOS; concurrent S3/Sync behavior; secure remote gateways. Those remain separate M0/M1/M2 gates, not hidden by green direct tool calls.

## Reproduce

From a Linux CI environment with Rust, Node, npm and package registry access:

```bash
bash scripts/mcp-direct-sdk-smoke.sh
```

The script starts a temporary Core, installs the pinned SDK in `mcp-direct-sdk/sdk`, runs direct client calls, writes machine-readable `AKAI_DIRECT_SDK_RESULT` lines and tears down the fixture. Never run against a personal Akula workspace. The optional test only executes automatically in a pull request whose title begins `AKAI-CONF-03`.

## References

- [Official TypeScript MCP SDK v1.x](https://github.com/modelcontextprotocol/typescript-sdk/tree/v1.x)
- [Official MCP 2025-06-18 lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle)
- [Official MCP 2025-06-18 Streamable HTTP](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)
- [MCP conformance runner](https://github.com/modelcontextprotocol/conformance)
- [Internal full-suite triage](https://github.com/Lalalalendia/Akula/blob/main/docs/agents/akai-conf-02-results-triage.md)
