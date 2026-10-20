# AKAI-CONF-02 — triage of official MCP 2025-06-18 results (2026-10-19)

**Status:** Evidence recorded, **not a conformance certification**.
**Implementation:** merged [PR #380](https://github.com/Lalalalendia/Akula/pull/380), squash `51a50adc0e3849281163de22e48391c5f61bb373` (diagnostic branch head `870ea7f3960fd415da07eef808fe490f4795d1f9`).
**Authoritative CI:** [run #39297147717](https://github.com/Lalalalendia/Akula/actions/runs/39297147717), artifact ID `12207878015` (`checks.json` for 27 invocations plus `conformance-summary.json` and raw logs).
**Harness:** upstream `@modelcontextprotocol/conformance@0.1.16`; `server-initialize` + `--suite active --spec-version 2025-06-18`. Real disposable Akula Core and temporary vault behind a **test-only loopback Bearer proxy**.

## Counts — what the runner actually says

- Standalone `server-initialize`: **1/1 SUCCESS**.
- Full filtered active suite: **5 SUCCESS, 21 FAILURE across 26 checks**.
- Main Rust, sync-engine, native frame, Windows socket tests and Main PR gate: green at the accepted commit.
- **Never report 5/26 as a protocol compliance percentage.** Some upstream scenarios require special fixture tools or optional methods that Akula did not advertise. Some of the runner's successes are false positives for intended tool behavior.

## Detailed classification

| Bucket | Official scenario(s) | Observed | Interpretation |
| --- | --- | --- | --- |
| Core handshake | `server-initialize` | PASS | Correct 2025-06-18 initialization for this official client on proxied transport |
| Core utility | `ping` | PASS | Valid server response |
| Advertised capability | `tools-list` | PASS | Valid tool definitions: **30 named tools** exposed, including `read_note` and `update_note` |
| Weak positive / test fixture mismatch | `tools-call-simple-text` | PASS from runner, **but `isError=true`** | Client requested `test_simple_text`; Akula returned `Неизвестный инструмент: test_simple_text`. Runner checks the presence of nonempty text, not success of requested tool |
| Weak positive / unknown-tool handling | `tools-call-error` | PASS from runner, **but `isError=true`** | Client requested `test_error_handling`; Akula returned an unknown-tool error. Error response shape is valid, but this is not a test of an implemented `test_error_handling` tool |
| Optional methods not advertised by Akula | `completion-complete`, `logging-set-level`, 5 `prompts-*` checks, 6 `resources-*` checks | 13 FAIL | Server currently advertises `capabilities.tools` only. Each failed with JSON-RPC `-32601` (unknown method). Do **not** add resource/prompt/completion features solely to make an optional test green |
| Special test tools/content/workflows | `tools-call-audio`, `tools-call-image`, `tools-call-mixed-content`, `tools-call-embedded-resource`, `tools-call-sampling`, `tools-call-elicitation`, `tools-call-with-logging`, `tools-call-with-progress` | 8 FAIL | Runner expects specific conformance-fixture tools, rich content types or request/notification exchanges. These cases are not a clean verdict on Akula's existing `read_note`/File Gate semantics |

**Total:** 3 directly meaningful core passes + 2 weak positive fixture-specific passes + 21 failed unsupported/fixture-dependent scenarios = **26** active checks. The standalone initialize repetition is separate.

## Requirements covered vs unproven

**Covered, narrowly:** one official client can initialize a loopback Akula server through the temporary credential proxy; `ping` and tool discovery work. Existing app tests separately cover BLAKE3 CAS, history and Writer Gate integrity.

**Not covered by this suite:** authenticated direct HTTP (because proxy inserts credentials), SSE/GET/Accept/chunk framing (proxy buffers responses), DNS-rebinding defense, full handshake state enforcement before tool calls, modern 2026 stateless request metadata/discovery, real `read_note` success through an official client, per-agent permissions, revocation, prompt-injection resilience, all mutating tools' conditional writes.

**False-positive guard:** future conformance harnesses must explicitly check `isError`, tool identity, content and side effects for `read_note`/other real Akula tools. Treat unknown-tool error-as-text `SUCCESS` in upstream scenarios as **UNVERIFIED**, not as green application functionality.

## Priority follow-ups

1. **AKAI-CONF-03 (P0):** direct server MCP protocol smoke with a test client able to supply Bearer, preserving direct transport headers and streaming. Check initialization, `tools/list`, `read_note`, invalid version, malformed requests, `Origin`/Host/auth precedence; record exact response bodies and official client SDK version.
2. **AKAI-PROT-02 (P0):** track capability advertisement and enforce lifecycle/session requirements for explicitly supported `2025-06-18`, including GET/SSE/notification and missing protocol headers. Do not implement unrelated optional prompts/resources to satisfy a generic test.
3. **AKAI-ADR-01 (P1):** evaluate thin official `rmcp` adapter vs current HTTP after conformance findings and real 100-client A/B, with no second File Gate. See [ADR PR #383](https://github.com/Lalalalendia/Akula/pull/383).
4. **AKAI-AUTH-01 / WRITE-01 (P0):** individual grants and CAS for all mutating operations remain separate safety gates.

## Source and reproducibility

- [Upstream conformance project](https://github.com/modelcontextprotocol/conformance) and exact `v0.1.16` tag.
- [MCP 2025-06-18 lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle) and [MCP tools](https://modelcontextprotocol.io/specification/2025-06-18/server/tools).
- Repo: `scripts/mcp-conformance-smoke.sh`, `scripts/mcp_conformance_proxy.py`, `scripts/mcp_conformance_report.py`.
- Archived [run #39297147717](https://github.com/Lalalalendia/Akula/actions/runs/39297147717) raw artifact ID `12207878015`. Retention is 30 days; this durable triage remains useful after the artifact expires.

**Release gate:** this investigation is a test/diagnostic milestone, **not** a completed M0 or a release-readiness badge.
