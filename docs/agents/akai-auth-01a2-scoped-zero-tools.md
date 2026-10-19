# AKAI-AUTH-01a2 — separate scoped MCP admission (zero-tool bootstrap)

**Status:** Draft stacked on [#407](https://github.com/Lalalalendia/Akula/pull/407); no per-vault MCP permissions are enforced by this stage. Parent program: [AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de).

## Why this step exists

The existing MCP HTTP listener accepts one global full-access token and delegates to the unscoped `tools::call(core, name, args)`. An issued per-agent token must **never** be treated as the new form of the old global token while the dispatcher remains unrestricted.

The new host-only `RunningServer::start_scoped_deny_all` is an **opt-in security fixture** with its own `AuthenticationMode::ScopedDenyAll`. The regular Akula `McpServer::apply` still starts **only the old legacy listener**; installed app settings, URLs, tools, contents and user vaults have not been changed.

## Contract

- Existing loopback `Host`, `Origin`, supported protocol-version and body size checks are unchanged and apply equally to both listener modes.
- Every HTTP request validates a 256-bit Akula-issued Bearer against the host-local SQLite credential store from #407. No `clientInfo.name` or JSON body field can authenticate a request.
- Scoped mode rejects anonymous, wrong, expired, revoked and *legacy global* credentials with 401. No fallback path to old shared-token validation exists.
- HTTP admission creates a typed `RequestPrincipal::Agent(AuthenticatedAgent)` containing a host-issued principal ID and policy epoch. The identity is request-local; **it is not yet a grant**.
- Only `initialize` and `ping` remain meaningful for authenticated scoped agents. `tools/list` advertises an **empty list**, and every `tools/call`, including mixed JSON-RPC batches, is denied before invoking any tool.
- The legacy listener remains unchanged and still exposes the normal tool registry. A scoped request cannot invoke the legacy unrestricted `SharedToolCaller`.
- Revocation is checked again on every subsequent HTTP request. This **does not** prove cancellation or revalidation for a request already admitted before a concurrent revoke; the current scoped router cannot read or write any document in any case.

## Negative acceptance

- Two host-issued agent Bearers connect to the same loopback scoped listener and complete `initialize`, but neither can list or execute note/workspace tools.
- Old shared Bearer, unknown key and missing key yield 401.
- After host revokes agent B, B immediately fails authentication on the next HTTP request while A still completes initialization.
- A mixed JSON-RPC batch cannot bypass the no-tool decision and cannot leak foreign workspace names, source paths or document snippets.
- Native Windows + Linux full Rust/Sync + Main PR gate on exact HEAD. Previous official direct SDK legacy test behavior must be retained; targeted SDK re-run when integrating real scoped grants.

## Next required step (AKAI-AUTH-01b)

The only safe route to actual agent reads is **trusted vault identity resolution + durable host-approved grants** using the canonical `plugins::Capability` and `GrantScope` vocabulary, with an explicit agent principal kind (not fake plugin registration). The host must tie `workspace` selection and the physical enrolled root to a stable `VaultId`. A path string, partial folder suffix or source-supplied UUID is not a vault credential.

Then the typed `RequestPrincipal::Agent` must be carried through `AuthorizedToolContext` into read/search and per-result filtering, with lease revalidation before returning bytes. Only after an official SDK two-agent/two-vault negative proof may `read_note` or search be advertised in the scoped mode. Writes and revoke/commit fencing stay blocked until 01d.

**Explicit non-claim:** This PR does **not** implement multiagent vault isolation yet because it provides **zero MCP tool privileges**, not differentiated resource privileges.
