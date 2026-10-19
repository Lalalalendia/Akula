# AGENT-PERF-07 — bounded admission grace for fully busy MCP sockets

## Prior measured findings

Merged #371 established real temporary Markdown 10/32/100-client load. Rejected [#372](https://github.com/Lalalalendia/Akula/pull/372) tested pressure-triggered `Connection: close` with a controlled same-run A/B. At 100 ordinary clients, it reduced completed reads from 2787.9/s to 2489.1/s and raised extra attempts (HTTP 503 + resets) per 600 reads from 672 to 987. That candidate was closed without merging despite green correctness CI. Do not repeat its assumptions.

## Narrow new hypothesis

The listener currently refuses the 17th socket immediately with HTTP 503 when all 16 slots are occupied by *active* requests and none are idle. If those active calls are microsecond/millisecond reads, a bounded grace period may let a completed call drop its slot and avoid a 503/retry storm.

- When all 16 slots are fully busy and no idle keep-alive is safe to reclaim, wait on the existing `freed` condition variable for **up to 5 ms**, then either admit the connection or return the same explicit 503. The listener retains its single bounded accept loop; no unbounded pending queue, extra thread or socket is created.
- The existing special case of reclaiming **idle** sockets stays unchanged (up to 500 ms and portable 50 ms cooperative polling). No File Gate, MCP protocol, writer priority, 4-reader limit, active tool cancellation, CAS or auth changes.
- Shutdown remains checked during wait; slow/incomplete request bodies must never be evicted to admit a newcomer.

## Controlled same-run A/B

`#[cfg(test)] disable_busy_grace` allows tests to run the old instant refusal (control) and bounded grace (candidate) within one Rust binary. For each of the 3 repetitions, test 10/32/100 clients against 64 temporary Markdown notes, with 6 reads per client, normal and `includeRevision=true`. Repetition 2 reverses mode order. Log all success counts, 503, transport resets, rate, p95/p99 and exhausted bounded retries.

## Decision gates

- Full Rust workspace, sync-engine, native Windows MCP HTTP and Main PR gate green; busy-active-socket regression must retain bounded 503 and no active eviction.
- At 100 clients, compare the median per-run **total extra attempts** (503 + resets) and completed throughput on the same runner. A reduction in one error type offset by another is not a win.
- Do not merge if the listener's added waiting causes materially worse p95/p99, runaway backlog, CPU/RSS or synchronized retry bursts. Three pairs are preliminary, not a full capacity proof.
- A 5-ms grace is not a fair queue, separate agent identities, remote permission isolation, or evidence of 100 *simultaneous in-flight* MCP calls.
