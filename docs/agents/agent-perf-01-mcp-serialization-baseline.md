# AGENT-PERF-01 — MCP synthetic concurrency baseline

## Observed code, not an inferred throughput result

MCP HTTP server `aquilum-app/core/src/mcp/server.rs` accepts up to 16 connections but `Endpoint::answer` takes a single process-wide `turn: Mutex<()>` around **all** protocol/tool calls **and HTTP response sending**. Even independent `read_note` calls serialize. This is both a tool-dispatch and possible slow-response blocking point.

Current `update_note` delegates to `write_retrying` with `Source::Agent`; its File Gate/history safety must not be bypassed for speed. The current single automatic retry on file conflict does **not** constitute an agent-visible expected-revision CAS contract.

## Reproducible first measurement

`cargo test -p aquilum-core mcp::server::tests::throughput_probe -- --ignored --nocapture`

The ignored test uses localhost, an ephemeral port, existing Bearer token validation and a **synthetic read-like tool** taking 12 ms per request. It makes **128 measured requests at each** of 1, 4, 8 and 16 clients, with one persistent `ureq::Agent` per client; the first connection setup is included (no warmup). All JSON-RPC responses are consumed and checked. Output `MCP_SYNTHETIC_SERIAL_BASELINE` reports wall duration, req/s, p50, p95 and p99, client count and errors=0 (other outcomes fail the diagnostic). No absolute latency threshold is used as a CI gate.

No user vault is opened, no Markdown is modified and no service is exposed beyond loopback. These numbers measure request/transport/endpoint serialization under fixed simulated work, **not** model inference, indexing, editing, disk fsync, S3 or real agent throughput. Record OS, CPU, Rust version/build, and at least three repetitions before comparing variants. The existing `runs_one_tool_call_at_a_time` remains the correctness regression test; this diagnostic is observational.

## Source-based operation classification (candidates, NOT concurrency approval)

| MCP route(s) | Existing behavior | Parallelism gate |
| --- | --- | --- |
| `ping`, `initialize`, `tools/list`, `dataview_syntax` | Static parsing and response work | Verify request/batch classification and stop semantics |
| `read_note`, `note_outline`, unsorted `list_notes`, `list_folders` | Direct filesystem reads/scans, plus workspace resolution | Prove consistent snapshots across File Gate, rename, trash and active-workspace changes |
| `search_notes`, `related_notes`, `find_notes`, `list_notes sort=links` | Indexed reads, often via `indexed_vault` | `indexed_vault` can call `open_guest` and wait for indexing: audit shared search/index state first |
| `note_history`, `read_version`, `list_trash`, `get_workspace`, `list_workspaces` | Read-like API over history/UI/search state | Audit interior locks, lazy work and snapshots before whitelisting |
| `create_note`, `update_note`, `set_frontmatter`, rename/move/delete/restore/version/trash changes | Mutation, File Gate/history or filesystem state | Preserve exclusive admission, permissions, explicit expected-revision checks |
| `switch_workspace`, `open_note` | UI side effects and workspace switching | Exclusive lane; never auto-classify as read-only |

Also note: `protocol::handle_message` supports JSON-RPC batches. A batch with **any** mutating or unknown member must not be admitted as a read-only batch. Avoid trusting a client-provided "read-only" label.

## Next design experiment, gated separately

1. Audit `Core`, index, HistoryService, `read_text` and File Gate lock ordering; write a read-only allowlist by **actual effects**, including cold guest-index behavior. Start with pure protocol routes and only then add safe file reads.
2. Prototype bounded concurrent read dispatch with writers exclusive and request-level lifecycle accounting. The current `turn` also protects `stop()` and sending responses; moving/removing it without a drain barrier changes shutdown behavior. Test stalled clients, cancellation and mixed batches.
3. Audit 16 connection slots, idle HTTP keep-alive occupancy and the 17th-client wait; distinguish socket admission bottlenecks from tool-lock throughput.
4. Prove real document expected-revision CAS, collision handling, conflict/HOLD semantics, permission and history integrity before enabling parallel writes.
5. Benchmark actual workspaces (cold/warm index, 1/10/50/100 clients; same versus independent documents, burst/sustained, p50/p95/p99, CPU/RSS, rejected requests and post-crash hash/history).

This PR changes **no production MCP synchronization, concurrency or mutation authorization** and makes **no high-throughput claim**.
