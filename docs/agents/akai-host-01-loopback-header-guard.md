# AKAI-HOST-01 — fail-closed loopback Host validation for MCP HTTP

**Status:** code in Draft PR; not verified until exact-head CI passes.  
**Parent:** [AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de), M0 protocol/transport hardening, precursor to M1 agent grants.  
**Based on:** [AKAI-CONF-03](https://github.com/Lalalalendia/Akula/pull/389) official TypeScript SDK two-client direct test (squash `3c599707cfa1886b0677245510c56f342c138585`).

## Vulnerability boundary and motivation

Before this change, Akula's MCP HTTP server bound `127.0.0.1` and validated `Origin` when present, plus its existing Bearer token. It did **not** validate the HTTP `Host` header. A browser can issue a request to an attacker-controlled DNS hostname that is subsequently resolved to `127.0.0.1` (DNS rebinding); relying on loopback binding and an optional Origin check alone is insufficient defense in depth. The Bearer requirement remains in force, so **do not claim a demonstrated credential bypass**.

The official MCP SDK transport guidance defaults to literal loopback Host allowlists; use that minimal policy here because Akula currently provides **only a local desktop listener**, no supported remote gateway.

## Change

- Bind remains exactly `127.0.0.1`; no cloud, wildcard interface, CORS change or new identity store.
- Require exactly one non-empty HTTP `Host` header. Missing, empty or duplicate `Host` fails closed with HTTP **400**.
- Allow only the literal case-insensitive `127.0.0.1`, `localhost` or `[::1]` host spellings; optional numeric port must exactly match the listener's actual bound port. Reject look-alike suffix domains, embedded scheme/userinfo, comma-separated values, malformed IPv6 authority, decimal/hex aliases and different ports with HTTP **403**.
- Reject untrusted Host **before reading a request body, authenticating the bearer, or invoking a tool**. Preserve current `Origin` and Bearer checks for accepted local Host values. `X-Forwarded-Host` is not treated as trusted authority.
- Support older local native callers with a bare loopback host (no port), but HTTP/1.1 requests with no Host will no longer be accepted. This intentional fail-closed compatibility change must be tested with the bundled stdio bridge and official legacy MCP SDK.

## Acceptance

1. Unit test loopback literal parser: accepted and rejected hostnames, ports, scheme/userinfo, IPv6 bracket formatting, DNS suffix spoofing and encoded numeric aliases.
2. Real HTTP socket test: allowed local Host values return 200; hostile values return 403, missing/empty/duplicate Host return 400. Tool invocation count does not increment for any denied Host, including requests bearing the correct token.
3. Re-run **AKAI-CONF-03** against real disposable Core + official `@modelcontextprotocol/sdk@1.27.1`: negotiated version `2025-06-18`, `read_note`, read revision, two competing conditional writes, and authorized/malformed protocol behavior must remain green. No credential proxy.
4. Native Windows MCP socket tests, Rust workspace, sync-engine, frame checks, Main PR gate on the exact SHA. Archive diagnostic output. Only after verified results may this Host change merge.
5. No assumptions about full 2025 conformance, modern MCP `2026-07-28`, public remote access or isolation of multiple per-agent grant identities.

## Follow-ups

- **AKAI-PROT-02:** legacy lifecycle/session enforcement and direct header/body/SSE tests; this change only limits Host authority, not HTTP framing.
- **AKAI-AUTH-01:** true per-agent credentials and scoped vault permissions; common Bearer in the fixture is not per-agent isolation.
- **AKAI-REMOTE-01:** if an authenticated remote gateway is later required, put its hostname policy at a separately reviewed gateway. Do not relax this desktop-only listener's loopback allowlist globally.

## Primary sources

- [Official MCP Python SDK: deploy and scale — Host allowlist and rebinding defense](https://py.sdk.modelcontextprotocol.io/run/deploy/)
- [Official MCP C# SDK: Streamable HTTP transport host-name validation](https://csharp.sdk.modelcontextprotocol.io/v2/concepts/transports/transports.html)
- [MCP 2025-06-18 security guidance](https://modelcontextprotocol.io/specification/2025-06-18/basic/security_best_practices)
- [Internal official SDK direct acceptance](https://github.com/Lalalalendia/Akula/blob/main/docs/agents/akai-conf-03-direct-sdk-core.md)
