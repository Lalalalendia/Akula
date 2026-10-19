# AGENT-PERF-01 — MCP synthetic concurrency baseline

## Observed code, not an inferred throughput result

MCP HTTP server `aquilum-app/core/src/mcp/server.rs` accepts up to 16 connections but `Endpoint::answer` takes a single process-wide `turn: Mutex<()>` around **all** protocol/tool calls and sending replies. Thus even independent `read_note` calls serialize at the endpoint. Current `update_note` delegates to `write_retrying` with `Source::Agent`; its File Gate/history safety must not be bypassed for speed.

## Reproducible first measurement

`cargo test -p aquilum-core mcp::server::tests::throughput_probe -- --ignored --nocapture`

Run against the normal host development binary on the same machine and toolchain. The ignored test uses localhost, an ephemeral port, existing Bearer token validation and a **synthetic read-like tool** taking 12 ms per request. For 1, 4, 8 and 16 concurrent clients it makes 4 requests/client and prints `MCP_SYNTHETIC_SERIAL_BASELINE` with throughput, p50/p95 and wall duration.

No user vault is opened, no Markdown is modified and no network service is exposed externally. The numbers measure request/transport/endpoint serialization overhead under fixed simulated work; they do **not** represent actual model inference, indexing, editing, disk fsync, S3 or real agent performance. No absolute timing threshold is used as a flaky CI gate. Existing `runs_one_tool_call_at_a_time` remains the correctness regression; the new test is purely observational.

## Next design experiment, gated separately

Do not blindly delete `turn`: MCP transport has stopping semantics and mixed read/write routes; File Gate and HistoryService have independent lock/order constraints.

1. Classify MCP methods/tools into immutable read-only vs mutating, and locate shared Core/index interior locks. Confirm that `read_note`, `list_notes`, `search_notes` really remain read-only under all options before concurrent admission.
2. Prototype read-only concurrent dispatch with bounded slots and order-independent responses; preserve token/origin checks, stop/drain semantics, per-request connection response ordering, and all mutation serialization. Measure p95 and throughput for read-only requests under controlled concurrency, then mixed read/write load.
3. Extend to true document-level CAS, bounded read snapshots, agent IDs/capability grants, conflict/HOLD diagnostics and edit batching **before** any 50–100 agent readiness claim.
4. Run real workspace benchmarks (cold/warm index, 1/10/50/100 clients; same vs independent documents, burst and sustained, p50/p95/p99, CPU/RSS, history/bytes hash and crash restart). Report environment and all rejected requests.

This PR deliberately changes **no production endpoint mutex or mutation authorization**, and makes **no high-throughput claim**.
