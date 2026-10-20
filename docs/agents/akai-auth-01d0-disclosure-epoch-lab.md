# AKAI-AUTH-01d0 — MCP disclosure-time epoch lab

**Status: test-only experiment, not installed MCP authorization.** The
`mcp::agent_disclosure_epoch_lab` module is compiled under `cfg(test)` only.
The real `McpServer::start_scoped_deny_all` still advertises no tools. There
is no production `HostPinnedReadRoot` constructor, trusted host Broker,
network read route or agent settings.

## Exact unresolved race

The merged [AKAI-AUTH-01b1a #432](https://github.com/Lalalalendia/Akula/pull/432)
reads a pinned file and revalidates both credential and grant before returning
a `CheckedNote`. But actual HTTP emission occurs after that function has
returned. If revoke commits during this gap, the current read candidate may
still be written. **Rechecking before returning is not a disclosure fence.**

The installed implementation must establish a total order between revocation
ACK and the **last attempted write of protected bytes to the TCP transport**.
A successful OS `write_all` does not guarantee remote receipt; the ACK order
contract concerns *server writes*, not delivery after TCP buffering.

## What this lab checks

For two existing host-private SQLite files (agent credentials and per-vault
grants), the test acquires a `BEGIN IMMEDIATE` writer transaction on each
in the fixed order **credentials first, grants second**. Under SQLite WAL,
reader queries still work while competing writers are blocked.

While both writer transactions remain alive it:

1. Re-authenticates the original bearer, including revocation and expiry.
2. Revalidates the original `AgentReadLease` epoch and registered vault.
3. Invokes an output writer inside the held pair of transactions.
4. Releases both transactions by rollback **only after the output closure
   returns**. A concurrently committed revoke using either SQLite database
   cannot ACK before this moment if all issuers participate in these same
   durable stores.

Independent writer connections in concurrent threads attempt both credential
and grant revocation during emission. Tests assert no ACK arrives while the
transport write is in progress and both revokes eventually complete after the
writer releases both locks. Revoke-before-lock denies without calling writer.

A custom output sink accepts the first four bytes then errors. The test
must classify this as `PartialDisclosureHold`; it is **never safe to retry
the complete payload**, treat it as unobserved, or send a generic denial
that falsely implies zero disclosure. Even a write returning an error with
**zero reported bytes** may have emitted unacknowledged bytes, so it is HOLD.
The writer also performs `flush()` **inside** the two DB transactions;
a flush error after an apparently successful write is also HOLD.

## Boundaries and attack-model limitations

- The experiment writes a small in-memory response, **not a live MCP socket**.
- It proves only cooperative SQLite ordering. Two databases **do not create
  an atomic distributed commit**. The ordering must be used consistently by
  all host-issued credential rotations, disables, grant updates, revocations
  and final disclosure. A future design needs an explicit lock-order spec and
  interruption/rollback behavior.
- A caller may replace the credential or grant SQLite/WAL/SHM namespace with
  same-user permissions. No database mutex or SQLite transaction grants
  privileged OS installer authority; physical file identity / sidecar
  ownership/ACL checks are still necessary.
- No physical root is verified by this lab. The real read route must use an
  independently privileged, host-attested local replica mapped to one
  `plugins::VaultId`. The remote `sync::VaultId` may be shared by distinct
  local replicas, as shown in [Staya #427](https://github.com/Lalalalendia/Akula/pull/427).
- The same-user attacker, separate device revocations, crashed writer,
  connection half-close, client-visible buffered data, and two different
  brokers with inconsistent epochs remain outside this experiment.
- Holding write transactions over long or blocked network writes is a
  latency/DoS risk. The production writer must bound reply size, socket
  write timeout, concurrency, and lock starvation; design remains open.

## Required tests

On the exact PR HEAD, Linux Core and native Windows MCP tests must execute:

```sh
cargo test --locked -p aquilum-core --lib mcp::agent_disclosure_epoch_lab::tests:: -- --nocapture --test-threads=1
```

Test cases:

- independent credential/grant revoke ACK cannot overtake emit;
- revocation of the credential *before* fence denies with no writes;
- revocation of the grant *before* fence denies with no writes;
- partial output fails with `PartialDisclosureHold`, no implicit retry.
- error with zero reported write bytes still classifies as HOLD;
- buffered writer flush fails inside the epoch and classifies as HOLD.

Full Rust workspace, MCP Windows, Main PR gate and existing File Gate
regression tests must also pass. Passing only validates a **candidate** for
a live disclosure guard. Actual release requires a Broker-attested root,
one local agent permission realm, native real TCP race tests on Linux and
Windows, actual two-agent official MCP SDK E2E, and a unified revocation
issuer that cannot bypass the fence.

Master plan: [AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de).
