# AKULA-AGENT-MASTER-01 — AI agents, MCP, memory and automation

**Canonical, fully detailed roadmap:** [Notion — AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de). Child of [AKULA-MASTER-INDEX-01](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b). Last verified: 2026-10-19; source baseline `main@6e7e8bd5903bad012513c60dfa0c494488c656ff`.

This is **one subprogram, not another independent backlog**. Plugin grant design belongs to PLUGIN-00–22; Sync and File Gate remain canonical owners of durability; workspace/memory belongs to AKULA-PROJECT-SPACE-01; IFC is covered by AK-SEC-IFC-10. AI must never become a prerequisite for local Desktop Beta.

## Product outcome

A local-first, provider-independent environment where separate AI agents can securely discover, cite and edit authorized Markdown, coordinate work and retrieve provenance-aware project knowledge. No unconditional remote exposure, model-controlled shell, global grants, silent CAS overwrite or parallel write bypass.

## Verified foundation

Merged: [#356](https://github.com/Lalalalendia/Akula/pull/356) baseline, [#361](https://github.com/Lalalalendia/Akula/pull/361) CI, [#364](https://github.com/Lalalalendia/Akula/pull/364) four allowlisted concurrent readers/exclusive writes, [#367](https://github.com/Lalalalendia/Akula/pull/367) existing symlink/reparse-point preflight (not TOCTOU-safe), [#368](https://github.com/Lalalalendia/Akula/pull/368) opt-in `expectedRevision` CAS for `update_note`, [#370](https://github.com/Lalalalendia/Akula/pull/370) 16-connection keep-alive reclamation / 503 backpressure, and [#371](https://github.com/Lalalalendia/Akula/pull/371) real temporary Markdown burst diagnostic. [#374](https://github.com/Lalalalendia/Akula/pull/374) remains a draft of saturation findings.

100-client / ordinary read fixture, medians of three repeats: **600/600 completed, 3395.6 req/s, p95 76.262 ms, 363 HTTP 503 and 178 transport resets** (metrics medians are separate). These are 100 HTTP client threads with read retries, **not 100 real LLM agents**. Reproducible [CI run #39273140774](https://github.com/Lalalalendia/Akula/actions/runs/39273140774), artifact ID `12197375243`.

Current `core/src/mcp/protocol.rs` has `PROTOCOL_VERSION = "2025-06-18"` and reflects the caller-provided version on `initialize`; version negotiation/conformance is not established. Server uses custom `TcpListener`/`httparse`, not RMCP.

## Work breakdown (unique identifiers and completion gates)

| Milestone | Work IDs | Required result | Prerequisites |
| --- | --- | --- | --- |
| M0 Protocol baseline (P0) | AKAI-PROT-01, OBS-01, ADR-01 | Official MCP conformance: 2025 legacy counter-offer, reject unsupported explicit HTTP headers, separately assess 2026 stateless protocol; measured transport ADR | None |
| M1 Local trust boundary (P0) | AKAI-AUTH-01, PATH-01, UX-01 | Per-agent/per-vault least-privilege grants, revoke-before-send/write, no-follow handle path operations, local connection UI; deny tests on Win/Linux/macOS | M0 + canonical Plugin permission broker |
| M2 Concurrent authoring (P0/P1) | AKAI-WRITE-01, WRITE-02 | Explicit CAS/identity for all mutating tools, structured conflict, preview/approval/idempotency; no loss under crash/restart | M1 + File Gate/History |
| M3 Transport and fairness (P1) | AKAI-PERF-01/02/03 | A/B custom HTTP/1.1 vs thin RMCP/Tower, 10/32/100 clients and mixed CAS load, bounded queues, no unbounded retries, measured p95/p99 and writer wait | M0; M2 for mixed writes |
| M4 Knowledge and citations (P1/P2) | AKAI-SEARCH-01/02, MEM-01 | Bounded search result with source/revision/range/provenance, optional hybrid index, safe derived memory and revocation | M1 + project-space/IFC contracts |
| M5 Jobs and AI workflow (P2) | AKAI-JOB-01/02, DEV-01 | Bounded durable tasks, progress, cancellation, no double-publish after restart, controlled handoffs, integration cookbook | M1 + M2 + M4 |
| M6 Remote and release (P3) | AKAI-REMOTE-01, REL-01 | Opt-in authenticated remote gateway with scoped grants, native E2E/red-team/rollback and release gates; no public desktop listener by default | M1–M5 and measured demand |

**Critical path:** PROT-01 → AUTH-01/PATH-01 → WRITE-01 → local Agent Alpha acceptance. OBS-01/PERF-01 and ADR-01 can run in parallel as diagnostics; do not merge production replacement just because a benchmark is faster. SEARCH/MEM/JOB and remote access do not block Desktop Beta.

## First PR-sized deliverables

1. **AKAI-PROT-01** — conformance matrix and *version-specific* handling: legacy `initialize` must counter-offer a supported version, HTTP explicit unsupported headers must reject, modern MCP needs a separate stateless implementation. [First-slice draft PR #377](https://github.com/Lalalalendia/Akula/pull/377).
2. **AKAI-OBS-01** — server-side counters plus paired 10/32/100-client transport A/B (keep-alive, close, bounded client); reuse findings from draft #374, do not create duplicate backlogs.
3. **AKAI-ADR-01** — RMCP/Tokio/Tower thin transport feasibility, features, MSRV/licensing, security/rollback and conformance comparison against the custom HTTP server.
4. **AKAI-AUTH-01** — per-agent identity and grant mapping to the existing permission broker, then adversarial revocation and foreign-vault tests.
5. **AKAI-WRITE-01** — extend existing CAS to one other mutating method and prove it before broadening coverage.

Each requires exact-HEAD CI, Windows/Linux/macOS where relevant, negative tests, stable data hashes/history, PR links, rollback, and progression `Proposed -> Ready -> In PR -> CI passed -> Merged -> Verified in installed build -> Released`.

## External sources

- [MCP 2026-07-28 specification release](https://blog.modelcontextprotocol.io/posts/2026-07-28/) and [MCP conformance](https://github.com/modelcontextprotocol/conformance)
- [Official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk), [MCP Tasks extension](https://tasks.extensions.modelcontextprotocol.io/specification/2026-07-28/tasks)
- [Google SRE: Cascading failures](https://sre.google/sre-book/addressing-cascading-failures/), [The Tail at Scale](https://research.google/pubs/the-tail-at-scale/)
- [AWS: Backoff and jitter](https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/), [Netflix Concurrency Limits](https://github.com/Netflix/concurrency-limits)
- [Tokio Semaphore](https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html), [Tower ServiceBuilder](https://docs.rs/tower/latest/tower/struct.ServiceBuilder.html)
- [OWASP LLM01 prompt injection](https://genai.owasp.org/llmrisk/llm01-prompt-injection/), [OWASP LLM06 excessive agency](https://genai.owasp.org/llmrisk/llm062025-excessive-agency/)

The **Notion canonical plan** contains full requirement/acceptance matrices, dependencies, constraints, roadmap details, risk register and project cross-links. This document is the code-adjacent development entry point, **not** a second master plan.
