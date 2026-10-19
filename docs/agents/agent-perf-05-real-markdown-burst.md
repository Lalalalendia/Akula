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
## Measured CI evidence — 19 October 2026

Source: Application CI run 39271521676 on PR head `d88b4b5`, required Rust workspace and Main PR gate green. Three sequential repetitions on one Linux CI runner. Each logical request can contain bounded retries, so `req/s` divides **successful logical requests** by end-to-end wall time, not TCP attempts.

| BLAKE3 revision | Clients | Logical requests / run | Median req/s | Median p95 | 503 per run | Transport resets per run | Exhausted |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Off | 10 | 60 | 3597.7 | 4.731ms | 0 | 0 | 0 |
| Off | 32 | 192 | 3013.2 | 26.614ms | 15–18 | 23–28 | 0 |
| Off | 100 | 600 | 2729.1 | 96.684ms | 437–502 | 175–205 | 0 |
| On | 10 | 60 | 3326.9 | 3.871ms | 0 | 0 | 0 |
| On | 32 | 192 | 2889.0 | 30.899ms | 19–27 | 17–30 | 0 |
| On | 100 | 600 | 2672.4 | 101.969ms | 479–517 | 188–208 | 0 |

Interpretation: throughput plateaus and then falls beyond 16 concurrent HTTP connections. For 100 clients, successful logical reads require hundreds of extra attempts caused by 503 and connection churn; this is transport admission overhead, not evidence that Markdown parsing or BLAKE3 is the main bottleneck. The modest difference between hash and non-hash modes is not a controlled microbenchmark of hashing alone.

Follow-up experiment: when the transport sees actual saturation, proactively close completed HTTP replies instead of leaving them as idle keep-alives. That should let queued fresh connections reuse slots without forced idle eviction. Keep the existing 16-socket and 4-concurrent-read limits, authentication and File Gate semantics; re-run the identical 10/32/100 benchmark before adopting. No claim about 100 concurrent in-flight tool calls.
