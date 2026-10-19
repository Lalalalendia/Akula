# AKULA-AGENT-LOAD-01 — experimental MCP concurrent reads

**Status:** opt-in prototype, NOT enabled in the ordinary installed or
signed Akula release. Baseline target: Akula/main
`1c72f829052b0507ce68a09b50d750ec6e35a967` (19 October 2026).

## Observed bottleneck, not a projected benchmark

Akula already implements an authenticated loopback HTTP/JSON-RPC MCP
transport, stdio bridge and note/search/edit tools. The existing MCP
`Endpoint.turn: Mutex<()>` serializes **every** request through one gate
even if separate agents read unrelated Markdown files. The connection
limit is 16, not 16 concurrently executing tools. A long read/search
request can hold unrelated reads behind it.

The MCP server also keeps this gate while sending its response. A slow
client or writer in the same admission queue can therefore affect
tail latency. None of these source observations is an independently
measured real-workspace throughput result.

## Narrow optional experiment

`aquilum-core/mcp-parallel-reads` and
`aquilum-native/mcp-parallel-reads` enable a **non-default**
read/write gate:

- Independently execute ONLY the explicit MCP tool names
  `read_note`, `note_outline`, `search_notes`,
  `get_index_status` and `dataview_syntax` with shared read access.
  `initialize`, `ping` and `tools/list` are also shared.
- All mutation tools (`create_note`, `update_note`, metadata updates,
  restore/move/rename/trash), navigation, unknown/future tools and
  malformed calls receive the **exclusive** turn. A JSON-RPC batch is
  read-only only if **every** item is approved.
- Retain exactly the same origin/token authentication, 4 MiB request
  limit, 16-connection bound, per-file CAS and File Gate/HistoryService
  source of truth. No new bypass or direct filesystem writer.
- Preserve default serialized mode in all existing packages/CI.

This is not per-document parallel writing: mutually exclusive writes
still serialize at MCP entry, and File Gate keeps its independent
contention and durability rules.

## Reproducible evidence

Dedicated Linux/macOS/Windows CI checks both default serialization and
opt-in HTTP parallelism. Deterministic barrier tests prove at least two
known independent reads enter together; a concurrent writer must not
enter before its reader leaves. Another test checks the exact allowlist,
mixed request batches and rejection of unknown/new tool names.
Existing server tests still prove that unknown/mutating calls remain
serial and that stop drains a running operation before refusing new work.

An intentionally **ignored**, opt-in synthetic HTTP measurement can be
run with:

`cargo test --locked -p aquilum-core --no-default-features --features mcp-parallel-reads measure_synthetic_http_mcp_read_lane -- --ignored --nocapture`

It uses 1, 4, 8 and 16 concurrent loopback clients, each issuing four
synthetic 30 ms `read_note` calls, and logs total elapsed time + requests
per second. It measures transport scheduler overlap with an artificial
tool, **not** actual Tantivy, filesystem or history workload performance.
Do not cite those numbers as real-world speedup.

## Release and security gates still missing

1. Real-workspace reproducible loads: 1/4/8/16 agents; independent
   reads, same-document CAS conflicts, writes in unrelated vaults,
   mixed read/write streams. Collect actual p50/p95/p99 latency,
   throughput, lock hold times and storage churn on all OS targets.
2. Fair admission under continuous reads: default `std::sync::RwLock`
   does not provide a portable strict writer-latency guarantee. Slow
   responses still hold a reader permit. Bound waiting, starvation,
   cancellation, response backpressure and shutdown must be proven
   before considering a release toggle.
3. Per-agent identity, permissions, scope and revocation: current
   `mcp.allow_write` is global, not a lease/grant per agent. TCP auth
   bearer token is not a project-level ACL. Concurrency is NOT a new
   security boundary.
4. Safe multi-document work: individual File Gate CAS/history commits
   are not a multi-file atomic transaction, and old `write_retrying`
   may re-evaluate a patch on one conflict. Do not imply agent task
   isolation or guaranteed semantic merges.

**Merge of this opt-in experiment does not authorize enabling the
feature in public builds or advertising 100-agent performance.**
