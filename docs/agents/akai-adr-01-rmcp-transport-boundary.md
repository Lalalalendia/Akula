# AKAI-ADR-01 — MCP transport boundary: custom HTTP vs official RMCP

Status: **PROPOSED — experiment authorization only, no migration decision**  
Owner program: [AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de) / milestone M0.  
Checkpoint: 2026-10-19. This ADR makes **no changes** to current File Gate, Sync, CAS, agent grants or any runtime.

## Decision under consideration

Should Akula keep evolving its custom `TcpListener + httparse` MCP transport, or implement a **thin, opt-in adapter** using the official Rust MCP SDK (`rmcp`)? The decision concerns **network protocol adaptation only**, not Akula's workspace, notes, history, synchronization, permissions or agent execution.

**Current recommendation: authorize an isolated proof of concept; decide only after A/B and conformance.** Do not replace the working HTTP server or add a default runtime dependency yet.

## Facts verified from code and upstream releases

| Topic | Baseline |
| --- | --- |
| Akula MCP | Localhost HTTP, Bearer/Origin checks, custom framing, 16 occupied TCP sockets, writer-priority execution with at most 4 allowlisted concurrent reads |
| Akula files | Persistent File Gate, history, source/file revision CAS (`expectedRevision` opt-in to `update_note`); these must not be reimplemented in an adapter |
| MCP legacy version | `2025-06-18`; [PR #377](https://github.com/Lalalalendia/Akula/pull/377) fixes false version advertisement and HTTP explicit-version gate |
| Conformance | [PR #380](https://github.com/Lalalalendia/Akula/pull/380): upstream conformance `server-initialize` **1/1 SUCCESS** on run `39294691440`, artifact `12207792559`; full 2025 active scenario matrix under CI; no complete spec compliance claim |
| 100-client read-only burst | [PR #371](https://github.com/Lalalalendia/Akula/pull/371), run `39273140774`: 600/600 logical reads after retries; ~3395.6 req/s, p95 ~76.3ms, median 363 HTTP 503 and 178 resets. Synthetic clients, not 100 autonomous LLM processes |
| Upstream official SDK | [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk) current release `rmcp-v3.5.1` (published 2026-10-05); `rmcp` workspace version `3.5.1`, **Rust MSRV 1.88**, **Apache-2.0**, Tokio async |
| Upstream transport | RMCP supports Tower `StreamableHttpService`, axum integration, stateless MCP 2026-07-28 requests, optional legacy session behavior and supported-version policy |

The Rust-version, licensing, feature size, binary size and backward compatibility must be verified against Akula's **actual** CI toolchain and installed Windows build; a green standalone demo is insufficient. Do not assert that HTTP/2 or rmcp automatically yields more than 4 parallel reads or fixes auth, session isolation, File Gate or CAS.

## Alternatives

### A. Continue custom synchronous HTTP/1.1

**Benefits:** minimal dependency churn, small current code scope, already validated on native Windows, full ownership of socket-admission behavior. **Costs:** continued responsibility for HTTP framing, transport compliance (content negotiation, SSE, sessions), Windows socket lifecycle, security patches, evolving protocol versions and client interoperability.

**Good enough when:** all targeted clients support legacy MCP and the only observed bottleneck is modest retry churn that a client-side in-flight cap resolves.

### B. Optional official RMCP adapter (preferred experiment)

**Boundary:**

`MCP client/IDE → RMCP/Tower/axum transport → protocol + identity/grant admission → same Akula Tool Router → same Core/File Gate/History/Sync`

**Rules:** use the **same** tool definitions and request semantics, not a second code path with weaker validation. New transport must normalize requests into the existing ToolCaller behind capability grants, retain denial-by-default and avoid exposing a writable public listener. Bridge old and new version semantics explicitly. Per-request factory must not create a fresh authorization universe or unlock a foreign vault. No second revision store or history layer.

**Benefits:** delegated protocol version negotiation, modern Streamable HTTP, Tower middleware ecosystem and potential multiplexing, community spec fixes. **Costs/risks:** Tokio/Tower dependency surface, runtime/MSRV compatibility, more code shipped or built, thread and queue boundaries, protocol-version downgrade semantics, SSE/framing differences, new request/session/identity bugs.

### C. Full replacement of Core tools by RMCP service

**Rejected.** It would duplicate Akula's file-write authority, workspace isolation, conflict semantics and agent permission model. An SDK is a protocol/transport component, not the owner of Akula files.

## Required A/B experiment before any production migration

Create a **separate optional transport-lab crate or branch**, feature-gated out of the default desktop build. Compare on identical commit, machine, source documents and request traces:

1. **Protocol correctness:** pinned official conformance scenarios for 2025-06-18 and 2026-07-28, including explicit supported-version matrix; score *only advertised capabilities* and separate required violations, optional nonfeatures, fixture-specific test tools and test-proxy artifacts.
2. **Security:** listen on loopback; valid/missing/revoked per-agent grant; invalid Origin and hostile Host; DNS rebinding; write lock, CAS conflict and cross-vault denial; cancellation / 503 / crash recovery. No remote access by default.
3. **Transport:** 10/32/100 client reads and 80/20 read/CAS writes, persistent and short-lived HTTP, fixed in-flight limits 8/16, same 64 Markdown fixture + large notes; p50/p95/p99, resets, explicit 503, retry amplification, accepted queue latency, writer wait, resident memory, CPU.
4. **Resource cost:** binary size, dependency inventory, default compile time, Rust MSRV and native Windows compile. Add no unbounded spawned tasks/channels or implicit retry of mutating tools.
5. **Operator UX:** enabled only by explicit configuration, version shown accurately, token/grant revocation semantics, clean rollback to existing transport, old clients remain supported per declared matrix.

**A/B acceptance:** no user-data loss, no grant escape, no weaker File Gate/CAS semantics, conformance improvements for claimed versions, and clear improvements on transport saturation or maintenance cost **without regression in writer latency or installed footprint**. Performance thresholds should be selected from measured distributions, not retroactively invented.

## Decision triggers

- If current HTTP/1.1 + bounded client in-flight removes most 100-client retry amplification, protocol defects are small and new SDK increases binary/runtime risk: keep A and revisit after client demand.
- If RMCP significantly improves direct MCP conformance and reconnect/p99 while keeping all security/footprint gates: pursue B as an opt-in transport behind the same Tool Router.
- If neither is materially better under legitimate usage: do not migrate for the sake of technology.

## Dependencies and explicit non-goals

Depends on M0 PROT-01 / CONformance result, M1 permission broker design, core File Gate policy. This ADR does **not** authorize remote gateway, OAuth/OIDC over public network, autonomous jobs, shell access, or changing project Sync/Roha/noq. Those retain their own milestones.

## Primary sources

- [Official Rust SDK repository and README](https://github.com/modelcontextprotocol/rust-sdk)
- [RMCP workspace Cargo.toml](https://github.com/modelcontextprotocol/rust-sdk/blob/main/Cargo.toml) — version, MSRV, licensing
- [RMCP v3.5.1 release](https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.5.1)
- [MCP 2026-07-28 specification](https://modelcontextprotocol.io/specification/2026-07-28)
- [MCP conformance framework](https://github.com/modelcontextprotocol/conformance)
- [Tower ServiceBuilder](https://docs.rs/tower/latest/tower/struct.ServiceBuilder.html)
- [Tokio Semaphore](https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html)
- [Google SRE: cascading failures](https://sre.google/sre-book/addressing-cascading-failures/)
- [AWS: backoff and jitter](https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/)
