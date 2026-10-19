# AKAI-AUTH-00 — tool authority inventory and agent-principal integration contract

**Status:** implementation foundation only; **no production per-agent authorization exists in this PR**.
**Program:** [AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de) / M1.
**Prerequisites:** merged direct real-client CAS/auth proof [#389](https://github.com/Lalalalendia/Akula/pull/389) and Host/DNS rebinding hardening [#393](https://github.com/Lalalalendia/Akula/pull/393).

## Code audit: don't mistake plugin identity for agent identity

The existing `plugins::PermissionBroker` has a closed `Capability` enum, typed `VaultId`/`NoteId` and `GrantScope`, deny-by-default checks, an `AuthorizationLease` with revision/epoch revalidation, and a SQLite `PermissionGrantStore` for a **registered plugin package**. The broker accepts trusted installer-created plugin identity, manifest, publisher and digest, which are NOT identities held by arbitrary MCP clients.

Current MCP starts with **one shared token** in `McpSettings`, compares it at HTTP admission, and dispatches `tools::call(core, name, arguments)` with no principal or authorization lease. Tool arguments can select any known workspace via path/address, including a different vault. `list_workspaces` returns names and paths; `get_workspace` reports active workspace metadata. A single shared Bearer, even with correct Host/Origin checking, cannot enforce per-agent least privilege.

**Reject:** calling `register_host_package` on an agent-declared name, constructing fake artifact digests, or treating requested `workspace` strings as proof of vault authority. These would create another trust-bypass surface.

## Scope of this first PR

`core/src/mcp/tools/authority_inventory.rs` is **test-only**. It classifies each of the 30 currently advertised MCP tools by:
- closed existing `plugins::Capability` vocabulary;
- read, write, destructive, UI-control or information-only effect;
- scope shape: one note, batch, source/destination vault, search results, all workspace metadata, or UI context;
- explicit approval requirement for destructive actions and UI manipulation.

Rust CI fails if a newly advertised tool has no entry, an entry duplicates a tool, or a high-risk operation is classified as unprivileged. This is an **authority review gate**, not live authorization. No production RPC behavior, token format or backward compatibility changes occur in AKAI-AUTH-00.

### Important deliberate conservative mappings

- `read_note` and `create_note` can affect **arrays of notes**; authorize each resolved note/path, not just the first `note`.
- `search_notes`, `list_notes`, `find_notes`, backlinks, and `list_trash` may expose snippets, paths, tags and history: filter **every result** after retrieving or before producing observable counts/errors.
- `move_note` can cross vault boundaries; source and destination need distinct host-controlled identity and authorization/commit checks, not a single string prefix test.
- `list_workspaces` reveals unrelated vault paths; return **only authorized vaults** once scoped mode exists. `get_workspace` and index status must be scoped as well. Never leak unfiltered candidate names through ambiguous-workspace errors.
- `empty_trash` is vault-wide destructive; `switch_workspace`/`open_note` are UI side effects requiring a direct human request/approval. Generic notes.read must not confer UI control.
- `dataview_syntax` is static syntax help and needs no note grant, but still must arrive through an authenticated MCP principal unless a separate public help endpoint is deliberately approved.
- No tool classification grants shell, arbitrary filesystem, network fetch or `AiInvoke`.

## Proposed AKAI-AUTH-01 trust architecture (NOT implemented)

`MCP HTTP/stdio → request credential → host-issued AgentPrincipalId → policy broker → resolved stable VaultId/NoteId → Tool Router → File Gate/History/Sync`

1. **Identity:** desktop host/UI creates each agent principal and a 256-bit random secret, never accepts `clientInfo.name` or `_meta` as authority. Hash/salt/token ID are kept in protected app-data, never in synced Markdown or logs. The SDK can send `Authorization: Bearer` per agent. Allow explicit revocation and rotation. Keep old global token only in a **separately labeled legacy full-access mode**; it must never serve as an automatic fallback/bypass when scoped mode is enabled.
2. **Broker reuse without role confusion:** reuse `Capability`, `GrantScope`, `HostContext`, `VaultId`, resource matching and lease revision concepts. Extend the canonical broker/consent storage to represent an **agent principal kind**, distinct from verified plugin publisher/package identity; do not insert fake plugins. Decide the database migration and generalization in an ADR before schema changes.
3. **Trusted resolution:** resolve `workspace` using the host index to stable vault identity **before** deciding grants; unknown/foreign vaults must return indistinguishable denies where feasible. Resolve each note, folder and search hit under the permitted vault, including destination vaults and deleted/history entries. No lexical path equality as a standalone authorization check.
4. **Execution and revocation:** construct a host-only `AuthorizedToolContext` passed to every tool. Re-check grant epoch before producing note bytes/results; for writes, atomically fence revalidation with File Gate transaction/commit. A lease check before asynchronous processing alone does not stop a concurrent revoke from racing a commit. Revoke acknowledgement must have an explicit transactional meaning.
5. **User consent:** read/search per-vault may be durable if explicitly approved; write/delete/UI control require stronger review and visible auditing. Never grant all 30 tools just because MCP was enabled; default new agents to no access. `tools/list` should not advertise unavailable tools or leak foreign-vault paths when grants restrict them.
6. **Errors and logs:** preserve existing `MCP_REVISION_CONFLICT` semantics and safe error shapes, omit secrets/absolute private paths and distinguish permission denial from CAS conflict only to authorized agents.
7. **No second File Gate:** ACL decisions must not replace content custody, CAS, durable history, one-time write tickets or sync ownership.

## Dependency-aware implementation plan

| Work ID | Deliverable | Required evidence |
| --- | --- | --- |
| AKAI-AUTH-00 | This test-only complete authority inventory | All 30 tools mapped; new tool without review fails CI; default runtime unchanged |
| AKAI-AUTH-01a | Principal storage + issuance/revocation contract, broker schema ADR | Unknown credential denied, no fake plugin identity, legacy bypass disabled in scoped mode |
| AKAI-AUTH-01b | Read-only one-vault vertical slice, tool router receives host-owned agent context | Two principals, A reads vault X only, B reads vault Y only, no path/workspace metadata leak, deny on revoked token |
| AKAI-AUTH-01c | Search/multi-note/history result filtering and workspace metadata | Partial access never leaks excluded names, snippets, hit counts or paths |
| AKAI-AUTH-01d | Scoped writes/approval and epoch commit fence | CAS 1 writer winner, one conflict; revoke-before-commit => no write, crash/restart no lost data |
| AKAI-AUTH-01e | UI and native installed OS acceptance, legacy migration | User can add/remove grants, view what each agent did, Windows/Linux native CI + installed build |

**Parallel permitted:** read-only per-agent tests and protocol lifecycle/SSE investigation. **Blocked until grant infrastructure:** remote exposure and unattended agent job orchestration.

## Mandatory negative matrix for release

- Unknown/expired/revoked credential; duplicate/ambiguous identity; shared-token legacy attempt in scoped mode.
- Agent A vault X vs B vault Y: read_note, includeRevision, list_notes, search_notes, history, trash, get_workspace/list_workspaces, ambiguous workspace errors, search snippets/counts.
- Multi-note arrays; move_note cross-vault source/destination; new file create, rename/delete, bulk empty_trash, UI navigation; symlink/junction/reparse point, TOCTOU and forged NoteId.
- Concurrent revoke vs in-flight read/search response and File Gate CAS write; same-revision conflict; crash/restart revalidation; no unbounded queues.
- Official TypeScript SDK direct with two **different bearer credentials**; native Windows + Linux, exact HEAD.

## Sources

- [Akula permission broker](../../aquilum-app/core/src/plugins/permission_broker.rs) and [persistent consent store](../../aquilum-app/core/src/plugins/permission_store.rs).
- [PLUGIN-02 — existing permission broker architecture](../plugins/permission-broker-v0.md); [PLUGIN-03 — SQLite consent](../plugins/permission-store-v1.md).
- [Official MCP authorization specification](https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization) and [MCP security best practices](https://modelcontextprotocol.io/specification/2025-06-18/basic/security_best_practices).
- [OWASP LLM06 — excessive agency](https://genai.owasp.org/llmrisk/llm062025-excessive-agency/).
- [Direct official SDK / File Gate acceptance](akai-conf-03-direct-sdk-core.md), [MCP Host hardening](akai-host-01-loopback-header-guard.md).

**Scope statement:** do not merge a claim that MCP access is per-agent limited on the strength of this documentation/inventory. The next meaningful security feature is AKAI-AUTH-01b after identities, consent and broker integration genuinely enforce it.
