# AKAI-CONF-01 — official MCP 2025-06-18 conformance smoke

Canonical roadmap: [AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de). This is a **real upstream conformance measurement**, not a declaration that the entire MCP implementation conforms.

## Test architecture

- Pin [modelcontextprotocol/conformance](https://github.com/modelcontextprotocol/conformance) package `@modelcontextprotocol/conformance@0.1.16`, which exposes `server --scenario server-initialize --spec-version 2025-06-18`. No floating `latest` dependency.
- Opt-in ignored Rust integration fixture in `mcp/server.rs` launches the **real Core and tool router**, using a disposable appdata directory and temporary Markdown vault. Actual MCP HTTP authorization remains enabled, binds only loopback and uses an ephemeral bearer. No production storage is accessed.
- The upstream CLI has no `--header` option. `scripts/mcp_conformance_proxy.py` is a **test-only loopback** proxy adding the disposable bearer to each request; it does NOT remove/disable the server's auth requirement.
- `scripts/mcp-conformance-smoke.sh` starts Core, proxy and pinned CLI, waits for explicit ports, records the upstream CLI exit and preserves official `checks.json`, proxy/fixture logs, and always cleans up.
- An opt-in step inside the **existing** `Application checks` Rust job runs only when the PR title begins `AKAI-CONF-01`. No additional CI runner or public listener.

## Truthful scope of evidence

The initial upstream result was **one passing `server-initialize` check (1/1)** on CI run `39294691440`, artifact `12207792559`. The next diagnostic runs **both** that stable smoke check and the upstream `active` suite filtered to `2025-06-18`, and summarizes every available official `checks.json` into `conformance-summary.json` with passing and failing counts. The larger suite is bounded to 240 seconds; a timeout means incomplete coverage. Even a passing initialization check does NOT prove full 2025 conformance. Because the proxy terminates HTTP and adds a bearer, it cannot validate direct authorization failures, SSE streaming, chunked bodies, HTTP keep-alive behavior, DNS rebinding, or malformed transport framing. Test those directly in a separate native transport suite. No claim is made for modern `2026-07-28`.

An upstream **conformance failure** is preserved in the raw `checks.json` and explicit machine summary; it does not silently fail unrelated product CI at this exploratory stage. A missing `checks.json` for either the smoke or the active suite fails the diagnostic job. See CI summary for which checks actually fail. The 240-second suite deadline may yield incomplete evidence and must not be reported as a complete specification audit. Never equate `test script exited 0` with `specification passed`; inspect individual check results.

## Exit gate for M0

1. Publish a machine-readable pass/fail matrix for every *claimed* protocol revision, with upstream conformance version, exact source SHA, fixture, scenarios and raw artifact links.
2. Add direct-server authenticated HTTP and STDIO checks, native Windows transport checks, and compatibility with official clients.
3. Resolve remaining gaps: legacy initialize lifecycle enforcement, batch semantics, unsupported 2026 per-request metadata, GET/SSE, and isolated-vault grants as their respective stages permit.
4. Decide custom HTTP vs thin RMCP on a measured ADR; do not replace File Gate, history or CAS.

Sources: [upstream conformance README](https://github.com/modelcontextprotocol/conformance); [2025-06-18 lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle); [2026-07-28 versioning](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning).
