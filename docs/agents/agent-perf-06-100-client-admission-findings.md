# AGENT-PERF-06 — 100-client admission findings and next gate

Source: [PR #371](https://github.com/Lalalalendia/Akula/pull/371), squash `d13901446ada01f5516897da2389c4a51335b924`. CI artifact `mcp-agent-perf-371-2ead58cfedd283c78e36c5a0e2f6f84f2f3e5950` / artifact ID `12197375243` from run `39273140774`, file `mcp-agent-burst.log`.

## Reproduced measurements

Three runs each, disposable Core workspace, 64 actual Markdown notes, 6 reads/client, 10/32/100 authenticated HTTP client threads, 75-attempt cap and jitter for read retries. These are **not** independent LLM processes.

| includeRevision | Clients | Reads completed per run | Median throughput req/s | Median p95 ms | Median 503 per run | Median transport resets per run |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| false | 10 | 60/60 | 4492.8 | 2.889 | 0 | 0 |
| false | 32 | 192/192 | 4143.0 | 21.541 | 15 | 17 |
| false | 100 | 600/600 | 3395.6 | 76.262 | 363 | 178 |
| true | 10 | 60/60 | 4548.0 | 2.765 | 0 | 0 |
| true | 32 | 192/192 | 3917.1 | 23.758 | 12 | 20 |
| true | 100 | 600/600 | 3389.3 | 80.384 | 367 | 192 |

No logical request exhausted retry budget. For ordinary 100-client reads, 363 503s + 178 resets is ~0.90 recoverable failures **per completed read** across a representative median run (note: medians are per metric, not a single coherent trace).

## What the evidence means

- **Correctness of eventual read completion is good under this narrow workload**. The test has no writes, changes of permissions, S3 or process crashes.
- **Transport admission is inefficient at 100 burst clients**. Connection cap remains 16, while tool read concurrency is 4. 503 and reconnect churn significantly exceed the 10/32-client baseline, and tail latency rises.
- Optional revision hashing did not establish an obvious material throughput penalty in these small files; results are inconclusive for large notes.
- This test cannot isolate whether 503s vs resets are primarily caused by the socket cap, idle eviction, stale keep-alive reuse or synchronized retry behavior. Do **not** raise MAX_CONNECTIONS from 16 to 100 as a blind fix.

## AGENT-PERF-06 next experiment (diagnostic first)

1. Add **server-side counters** with no client/token identifiers: accepted connections, reclaim attempts/successes, 503 saturated/active, idle evictions, active/idle sockets, requests admitted to the bounded read/write dispatcher, queue wait histogram. Ensure bounded memory and no user document content in metrics.
2. Add deterministic **A/B transport strategies** on the same 100-client fixture:
   - persistent keep-alive with current recovery (control),
   - short-lived `Connection: close` after each read,
   - persistent keep-alive plus *client-side bounded concurrent in-flight requests* (e.g. 8/16).
   Fix request counts and documents across variants, compare 503/reset amplification, throughput, end-to-end p95/p99, CPU and RSS.
3. Add mixed 80% read/20% conditional-CAS **write workload** using distinct documents and same-document contention. Validate exactly one CAS winner per expected revision, no lost updates, writer wait latency and history/file hash invariants; keep this independent of the read-only burst test.
4. Design request-aware and **authenticated** admission, explicit `Retry-After` on overload, fixed bounded queue and writer fairness. Test slow bodies, slow clients, cancellation and shutdown. Only consider changing the 16 connection cap if measurements show a net benefit.

## Acceptance

Diagnostic tests must not become performance-threshold CI gates. Never use retry of mutating tools without a fresh expectedRevision. Changes to admission that affect production need native Windows and Linux runs, 100-client A/B repeats, and real Core/File Gate/history correctness before merging.
