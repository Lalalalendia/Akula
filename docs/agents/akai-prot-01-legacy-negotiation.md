# AKAI-PROT-01 — Legacy MCP version negotiation, compatibility-first slice

**Canonical plan:** [Notion AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de). **Scope:** narrow corrective first slice of M0; no claim of full MCP conformance or new-era support. Run Linux, Windows and sync-engine CI on the exact PR head before marking verified.

## Problem / observed implementation
`mcp/protocol.rs` advertised a hardcoded legacy version `2025-06-18` but returned arbitrary `initialize.params.protocolVersion` strings verbatim (including `2026-07-28`), thereby falsely claiming protocol support. HTTP accepted an explicit arbitrary `MCP-Protocol-Version` header and could execute an unrelated tool despite the unsupported version.

## Contract of this change
- **Handshake-based MCP 2025-06-18:** when a well-formed client requests exactly `2025-06-18`, the server responds in kind. If it offers another valid version, the server counters with its **actually supported** version, `2025-06-18`. This is the 2025 lifecycle rule, **not** a JSON-RPC error or a silent protocol upgrade. A client unable to speak it should disconnect.
- Malformed `initialize.params` missing `protocolVersion`, `capabilities`, or `clientInfo.name/version` returns JSON-RPC `-32602` rather than a false successful initialization.
- **HTTP:** explicit version header must be exactly `2025-06-18`. An unsupported, blank, or duplicated `MCP-Protocol-Version` header receives **HTTP 400 before any MCP tool invocation**. Check loopback Origin / bearer first, preserving 401/403 responses. The stdio-to-HTTP bridge identifies itself as `2025-06-18` and includes the legacy Streamable HTTP Accept values.
- No `2026-07-28` stateless discovery, `_meta` negotiation, `server/discover`, task-extended requests, request metadata or HTTP/2 claims. Those require a separate spec-compliant adapter and conformance test.
- Missing HTTP version headers remain accepted on the existing loopback bridge for transitional compatibility. Under the official 2025-06-18 transport, absent version must default to `2025-03-26` if there is no other way to determine it; Akula's acceptance is therefore **still a documented conformance gap**, not proof of a complete 2025 server.
- The old handler also accepts JSON-RPC batches, permits tools before successful initialize/initialized, and does not expose a Streamable HTTP GET/SSE path. No broad change to those behaviors in this narrow PR; each must be scored in the official conformance report.

## Regression tests
1. Valid legacy request: supported version returned, tools capability stays intact.
2. Mismatched 2025-03-26/2025-11-25/2026-07-28/arbitrary client offers: counter-offer exact 2025-06-18, never claim modern.
3. Missing, non-object, malformed capabilities/clientInfo and non-string version: error -32602.
4. HTTP authorized request with proper header works; unsupported, empty and duplicated version headers are HTTP 400 and **do not execute a tool**.
5. Without bearer, an unsupported version still returns 401; other origin/bearer tests and stop/overload tests remain green.

## Next M0 acceptance / open
- Run official [MCP conformance](https://github.com/modelcontextprotocol/conformance) against an isolated disposable Core, record scenario-level pass/fail, required vs pending/extension, `2025-06-18` baseline and 2026-07-28 intentionally unsupported.
- Introduce a correctly versioned thin RMCP adapter only after an ADR; preserve File Gate/CAS/custody and the local listener authority boundary.
- Do not conflate a clean Rust workspace test with a full compliance verdict.

## Primary normative sources
- [MCP 2025-06-18 lifecycle and version counter-offer](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle)
- [MCP 2025-06-18 Streamable HTTP version header behavior](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)
- [MCP 2026-07-28 versioning and UnsupportedProtocolVersionError](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning)
- [MCP conformance framework](https://github.com/modelcontextprotocol/conformance)
