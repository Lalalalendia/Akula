# AKAI-HOST-FASTPATH-V1a — first real scoped MCP HTTP read (CI fixture only)

## Implemented integration

A real localhost HTTP/MCP request/response path in disposable Rust tests, reusing
the existing AgentCredentialStore, AgentVaultGrantStore, HostPinnedReadRoot::read_checked,
MCP HTTP admission/dispatch, and pinned OS-relative Markdown reader.

Two agents and two independent roots will exercise:

- tools/list: only read_note for a principal with an active read grant, no tools for others.
- Agent A reads exact X/note.md; B reads Y/note.md; both receive their revision.
- Cross-vault read, ungranted agent, path traversal and write tools are refused.
- Revoke an agent vault grant: the tool disappears and later HTTP reads fail.
- Revoke an agent bearer: HTTP admission returns 401.

## Hard trust and activation boundary

Positive routing lives exclusively behind #[cfg(test)] in Core, through a
test-only ScopedReadFixture. It accepts only self-issued disposable fixture
registrations and ephemeral Markdown directories; no production code can
instantiate this fixture or call its protocol adapter. Normal scoped agents
continue to be denied by the original handle_without_tools route.

The fixture does not validate an independently signed and installed Host Broker,
a protected HostInstallationId/ReplicaId/VaultId ledger, or the entire 128-bit
Win32 FILE_ID_INFO bridge. A self-minted test receipt is not valid production
authority. A successful read_checked call is not yet an atomic final HTTP
send/revocation fence. No production agent may receive NotesRead from this change.

## Acceptance

- Native Core test scoped_host_v1_real_http_two_agents_read_and_revoke on Linux/Windows.
- Existing direct MCP transport, Core File Gate, default scoped deny-all tests pass.
- Application CI and required main PR gate pass at exact HEAD.
- No new installed binary, standalone Broker opcode, or production permission.

## Next actual delivery

Replace the disposable fixture's receipt with signed installed Broker evidence.
Bind host install, local replica and canonical plugins::VaultId to the live pinned
root, and fence revocation through the final HTTP response. Then enable exactly
one per-agent read_note via the preexisting Tool Router after E2E proof.

[Fastpath plan](https://app.notion.com/p/40032a84beec8156a28ae90ef04c865f)
[Agent master](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de)