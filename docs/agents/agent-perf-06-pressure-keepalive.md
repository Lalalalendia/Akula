# AGENT-PERF-06 — pressure-aware HTTP keep-alive experiment

## Why

Real temporary Markdown burst, merged PR #371: at 100 clients, 600 successful logical reads required 437–502 HTTP 503 responses and 175–205 transport resets per repetition. Median rate was 2729.1 logical reads/s, p95 96.684 ms. Ten clients had zero overload and 3597.7 reads/s. The likely bottleneck is the 16-slot socket lifecycle, not Markdown parsing or BLAKE3.

## Change under experiment

- Preserve 16 TCP socket slots, four allowlisted concurrent readers, writer priority, File Gate, CAS, and Bearer/Origin checks.
- When accept actually reaches the socket cap, remember a monotonic timestamp. For 300 ms after that saturation, send `Connection: close` on completed HTTP replies even if the client requested keep-alive. Normal traffic without saturation retains keep-alive.
- Never close an in-flight tool operation. Use existing RAII cleanup when a completed response ends.
- This is an availability tradeoff. Local attackers could induce saturation and extra client reconnects; secure per-agent ingress quotas are a separate task.

## Acceptance

- All existing Linux Rust, sync-engine, main-gate and native Windows socket tests must stay green.
- A deterministic unit test must prove pressure-mode expiration, and the 17th request behind 16 idle connections must receive a successful response and a close header.
- Re-run the same three real-Markdown burst repetitions (10/32/100 clients × normal/revision reads), archive raw 503, resets, req/s, p95/p99, and compare to PR #371 on a comparable runner.
- Do not merge on a throughput metric alone. Prefer fewer retries and resets without materially worse p95 or lost request correctness. If it regresses, reject or revert the prototype.

## Limits

100 HTTP client threads are not 100 simultaneously running LLM agents. Current server still admits only 16 active TCP connections and at most four parallel MCP reads. This change does not solve authenticated fairness, writes to many documents or S3 scaling.
