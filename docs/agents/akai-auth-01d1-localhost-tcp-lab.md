# AKAI-AUTH-01d1 — localhost TCP disclosure epoch (test-only)

**Parent:** [AKAI-AUTH-01d0 PR #458](https://github.com/Lalalalendia/Akula/pull/458) was merged into `main` as [`6da43aa`](https://github.com/Lalalalendia/Akula/commit/6da43aad57f4b92bb0272cf8dd89a76bb22db11a). This child PR adds only two test-only cases to its module and this document; it does not enable production MCP disclosure.

## Testable boundary

The current live scoped MCP listener still routes agent messages to `handle_without_tools` and returns no protected notes. The merged pinned-read helper has no production constructor from an untrusted root. Do not turn this lab into a real tool until independently protected host broker enrollment, local `plugins::VaultId` mapping and authorized physical root are accepted.

The previous parent lab passed in-memory fake writers to a *test-only* `disclose_cooperative` function. This child passes an **actual `TcpStream` accepted on IPv4 loopback** to that function and writes a synthetic HTTP response.

It verifies the following ordering, for each of two independent SQLite writer connections attempting to revoke the bearer or vault grant:

1. Start the credential and grant `BEGIN IMMEDIATE` writer epochs in fixed order.
2. Reauthenticate the original bearer and grant inside both epochs.
3. Start two independent revoke writers and record their intent to revoke. This rendezvous does **not** prove that each thread has already entered SQLite's blocking lock acquisition.
4. Emit a complete synthetic HTTP reply over localhost TCP. A real client performs `read_exact` and confirms the received bytes **while the locks are still held**.
5. Assert neither revoke has ACKed before receipt.
6. Release both epochs after `write_all`/`flush`; only then may revokes ACK. Old bearer and grant must no longer authorize.
7. With a credential revoked before the fence, invoke the same TCP emitter: writing must not happen, and a `shutdown(Write)` yields an empty read on the client.

The test is intentionally on Linux and native Windows through the existing `mcp::agent_disclosure_epoch_lab::tests::` filter. No workflow or production code is changed. Both tests are `#[cfg(test)]`.

## What this does NOT prove

- The generated response is **synthetic HTTP bytes** written through a standalone socket; this is NOT a live Akula MCP request or official MCP SDK conversation.
- The test does not prove client delivery ordering in the general case: small localhost data can be read before ACK, but on other networks a delayed read may happen after revoke. Only the server's last protected write can be fenced; kernel and network delivery cannot be rescinded.
- The two SQLite stores are cooperative, same-user-modifiable data, not an independently attested installer credential/grant issuer. A hostile process can replace DB/WAL/SHM or bypass the transaction protocol.
- There is no root identity validation, `AuthorizedToolContext`, real `read_note`, Plugin PermissionBroker integration, or independently issued device/vault identity.
- No HTTP server change or production response-fence integration; no claim of Windows hostile-process protection or multi-device global revocation.
- Revoke-thread start signals are not instrumentation of SQLite lock contention. The test observes the absence of ACKs during the protected emit, not a formal proof that both revocations were already blocked in SQLite.

## Acceptance

1. **Done:** Parent #458 passed its six tests and was separately merged.
2. At exact child HEAD, prove **both real-TCP tests pass on Linux and Windows**; prove 8/8 total AUTH-01d0/01d1 tests pass, not only unrelated server or grant tests.
3. Rust workspace and Main PR gate remain green; no active scoped tool gains a read path.
4. Failure must remain visible and Draft until fixed; no merge because a synthetic localhost socket worked.

Follow-up after Broker readiness: actual MCP HTTP/socket connection with authenticated agent A and B, independent registered physical vaults X and Y, revoke before **actual** protected HTTP disclosure, isolation of tool-list/search/error metadata, and official MCP SDK end-to-end verification.

[AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de).
