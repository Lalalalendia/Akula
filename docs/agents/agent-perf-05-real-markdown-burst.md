# AGENT-PERF-05 — MCP 10/32/100-client burst on real temporary Markdown

## Baseline and motivation

PR #364 provided bounded concurrency of four known-safe readers, #368 added optional raw-byte BLAKE3 revision preconditions for writes, and #370 introduced portable idle keep-alive reclamation and bounded 503 admission with 16 live tool-connection slots. None of those tests established 100-agent throughput or fairness.

## Experiment

- Opt-in ignored Core integration diagnostic; only a PR title beginning `AGENT-PERF-05` runs it in the existing required-tests Linux job. No new CI runner is scheduled and no production code is changed.
- Each run creates an isolated temporary workspace with 64 real Markdown files, registers it with Core and exposes the real `mcp::tools::call` through the localhost MCP HTTP server. All traffic uses the same Bearer token validation and JSON-RPC handling as the application.
- Perform two API modes: ordinary `read_note` and `read_note(includeRevision=true)`, the latter computing a BLAKE3 revision from the source document. For each mode, run 10, 32 and 100 concurrent clients, six sequential reads per client.
- Each client has its own persistent `ureq::Agent`. A logical read may transparently reconnect/retry after explicit `503` or a transport reset; retry is safe for reads only. Retries are bounded at 75 and use deterministic jitter. No writes or user files are touched.
- Record completed logical reads, number of `503` responses, transport resets, exhausted retries, wall throughput and end-to-end p50/p95/p99 including backoff. Each group is repeated three times on the same CI runner; archive raw logs and runner metadata.

## Acceptance and interpretation

- Test must compile and complete all logical reads. An exhausted request is a diagnostic failure. Do **not** gate on absolute req/s or latency across runners.
- High total `503` may still yield eventual success but indicates agent reconnect pressure and unfair/adverse admission, especially above 16 connections.
- Compare `includeRevision=false` vs `true` **on the same runner** to quantify hashing cost; compare 10/32/100 to locate throughput flattening and tail-latency growth.
- The 100 clients are HTTP client threads, not 100 real LLM processes. This measures MCP HTTP and file reads only, not inference, S3, File Gate CAS, history changes, arbitrary document sizes, model rate limits, or crash recovery.

## Next decision

If p95 and retry amplification are excessive at 100 clients, do not raise `MAX_CONNECTIONS` blindly. First examine request-level admission, fairness and per-agent quotas, authenticated ingress, keep-alive pressure, and bounded backoff or `Retry-After` semantics. Keep existing four-reader/one-writer safety invariants.