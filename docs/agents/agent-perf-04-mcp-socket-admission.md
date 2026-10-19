# AGENT-PERF-04 — bounded MCP socket admission and idle keep-alive reclamation

## Observed prior behavior

`Endpoint` limited connections to 16, but `Slot::wait` blocked the **single accept loop** until a slot was dropped. All 16 HTTP/1.1 connections could sit in keep-alive for up to 60 seconds *after finishing their tool call*, preventing another agent from making progress. The old test explicitly confirmed the 17th request waited until a caller closed a socket.

## Proposed behavior (draft)

- A connection retains its normal 60-second idle timeout when capacity is available. Existing clients may reuse keep-alive normally.
- After a complete response is sent, register its idle socket with the slot tracker **only if the HTTP parser has no buffered pipelined bytes**. While an active request is reading headers/body, calling a tool or writing its reply, its socket is not advertised as idle.
- At the 16-connection cap, accept may reclaim **one idle** keep-alive socket using `TcpStream::shutdown`. Admission waits **at most 250 ms** for the worker to release that slot. A concurrent parse must check the evicted flag before invoking any MCP tool.
- If there is no idle socket (all 16 slots active, partially framed, or writing responses), immediately send a bounded HTTP `503 Service Unavailable` with `Connection: close`. Do **not** cancel active reads or writers or wait 60 seconds in the accept loop.
- `Slot` RAII owns the connection count, removes its own idle registration and notifies waiters at cleanup; no new unbounded thread pool, no increased connection cap and no change to File Gate, S3, CAS or bearer/origin validation for admitted tool calls.

## Tests

1. Establish 16 authenticated keep-alive connections and wait until all are registered as idle; an authorized 17th call must get HTTP 200 promptly and the selected idle victim must be disconnected; count stays bounded.
2. Hold 16 connections with deliberately incomplete authorized request bodies; a 17th request must receive explicit HTTP 503 with a closed connection and the original 16 must remain untouched. Completing a previous body still reaches JSON-RPC.
3. Existing keep-alive reuse, stop/503, authentication and mixed read/write admission tests must remain green on Rust workspace and sync-engine.

## Deliberate scope and unresolved risks

- The 16-socket limit remains: there are **not** 100 concurrent tool requests. This is backpressure/reclamation, not a throughput result for 100 agents.
- Idle sockets can be evicted even if the owner wanted to reuse them; well-behaved clients must reconnect after EOF or reset. MCP tool invocations *already admitted* must never be evicted.
- Saturated connections are answered before request authentication; this does not grant tool access, but permits a local untrusted process to trigger idle-connection churn. A future secure per-agent quota and authentication-aware ingress should address this availability threat.
- Waiting for an idle socket to release is bounded, but worst-case serialized overload acceptance under hostile local flood needs stress testing, CPU/RSS monitoring and a final network implementation review.
- Slow headers and slowly trickled request bodies can still occupy up to 16 sockets for a long time; future transport-level deadlines, authenticated admission and fair queues need independent work.
- Distinguish successful request execution from connection occupancy and reconnect overhead in later 32/100-client benchmarks. Do not claim 100-agent readiness on this PR.
