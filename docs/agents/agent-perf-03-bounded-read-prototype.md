# AGENT-PERF-03 — bounded MCP read-only admission (draft experiment)

Status: **experimental branch; not production-approved**. This work follows the unchanged serialized baseline of AGENT-PERF-01 and the opt-in CI measurement gate of AGENT-PERF-02.

## Change under review

The HTTP endpoint previously held one global mutex around every `protocol::handle_message` call **and response send**. This prototype replaces that guard with a bounded, writer-preferring admission gate:

- At most **4** read-allowlisted tool calls active at once (still only **16** live TCP connections).
- Writers and unclassified operations remain strictly exclusive with all reads and other writes.
- Once a writer waits, no new readers enter until queued writers have run (writer preference).
- An admission permit covers dispatch **and the HTTP reply**, preserving the previous stop/drain boundary.
- `stop()` wakes blocked calls, drains admitted calls, and leaves queued work unable to mutate storage.

The allowlist is evaluated from the **entire JSON-RPC body** after HTTP token/origin admission, not from a client-provided privilege. Entire batches go exclusive if any member is unknown, malformed or mutating.

## Explicit read allowlist

| Method/tool | Reason included |
| --- | --- |
| `initialize`, `ping`, `tools/list` | Protocol metadata and fixed definitions |
| `read_note` | Uses `requested_vault` and direct `read_text`; does **not** call `open_guest`, File Gate or history mutation |
| `note_outline` | Uses `requested_vault`, `read_text` and heading extraction |

The initial allowlist intentionally **excludes** `search_notes`, `find_notes`, `related_notes`, `list_notes sort=links` (may initialize guest indexes); `note_history` and `read_version` (history synchronization needs independent review); `get_workspace` and `list_workspaces` (application state); and all UI, file, trash, version and frontmatter changes. Unrecognized tools remain exclusive.

## Evidence and acceptance

- Protocol tests: classify benign requests/batches; mixed and unknown batches fail closed.
- Endpoint tests: true overlap of independent mock reads, hard cap of 4, zero overlap with mock writes.
- Gate test: a waiting writer is served before later arriving readers.
- Shutdown test: 4 accepted reads complete and blocked mutation returns 503 with no tool execution.
- Existing MCP HTTP auth, origin, keep-alive, 16 connection cap, sequential writes and stop tests must stay green.
- The ignored synthetic benchmark runs both `route=exclusive` (unknown synthetic tool) and `route=read-only` (`read_note` routed to the same mock function) with **128 requests at each of 1/4/8/16 clients**. CI runs three attempts on one runner and archives raw output.

No claim about actual workspace throughput or 10–100 agents is justified by mock results. Before approval, prove safe concurrent reads against **actual local Markdown** while File Gate, history, indexing and workspace switching run, including post-run byte hashes, revision consistency, and crash/restart behavior. Stop/release must be tested under slow clients.

## Known limitations and follow-ups

1. **Keep-alive socket slot occupancy**: 16 idle connections can prevent a 17th request from being serviced until a slot releases; a 60-second idle socket timeout is not a request queue.
2. **Readers can hold the permit during slow response sends**: writer preference prevents new readers but cannot interrupt 4 already active reads; measure max write wait and slow-reader behavior.
3. **True agent CAS is still missing**: `write_retrying` automatically retries one stale-read conflict. Agent edits must eventually accept an explicit `expectedRevision` to avoid stale semantic changes.
4. **Real file/index consistency**: shared `Core` interior lock safety is not equivalent to correct cross-operation snapshots. Keep this draft until integrated fidelity tests are green.

Do not merge this prototype based only on synthetic speedup.
