# AKAI-AUTH-01a — host-issued MCP principal and credential foundation

**Status:** first implementation slice in Draft PR; **identity storage only, not agent-vault authorization**.
**Parent:** [AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de).
**Dependencies:** [#389](https://github.com/Lalalalendia/Akula/pull/389) direct SDK proof, [#393](https://github.com/Lalalalendia/Akula/pull/393) Host guard, [#400](https://github.com/Lalalalendia/Akula/pull/400) tool authority inventory.

## Security boundary (ADR 01a, first slice)

Akula has **two unrelated principal kinds**:

1. **Installed plugin:** trusted host registers a verified package, publisher, digest, declared capabilities and grants through `plugins::PermissionBroker`.
2. **MCP agent:** an arbitrary local process, only identified by a fresh credential issued by Akula's **trusted host**. MCP `clientInfo.name`, user-provided `workspace`, JSON `_meta`, and plugin manifest claims have **zero authentication value**.

Never call `register_host_package` for a made-up agent plugin. Preserve the existing `Capability` vocabulary and model separate principal-kind storage instead of grafting agent credentials onto publisher/digest fields.

The Core module `mcp::agent_principals` provides typed `AgentPrincipalId`, `AgentCredentialId`, `AuthenticatedAgent` (identity **only**), and an independently stored SQLite credential registry. No MCP network route can reach this module yet. This prevents accidentally accepting a credential without an enforced vault grant. The current single global Bearer and user-facing settings remain unchanged.

## Token and durable data contract

- Host-issued bearer: `akmcp1.<credential-uuid>.<64 lower-case hex chars>`. The secret is 32 bytes from `ring::rand::SystemRandom` (OS CSPRNG); the UUID selects a record but conveys **no permissions**.
- Only a 32-byte independently randomized verifier salt and HMAC-SHA256 verifier over the UUID + secret are persisted. This is safe against offline guessing at 256-bit secret entropy, **not** a password-hashing recipe for user-chosen low-entropy passwords.
- Issuance returns an `IssuedAgentCredential` with a `take_bearer_once()` handoff: the second extraction returns `None`. The returned `OneTimeAgentBearer` wraps zeroizing memory **without** implementing `Debug`, `Clone`, `Serialize` or `Display`. Do not expose a bare `Zeroizing<String>` as an API token: that generic wrapper itself implements `Debug`/`Clone`/serialization. Trusted host code must reveal the bearer once, never persist it in settings, sync or logs; explicit copies made by callers remain caller responsibility.
- Dedicated host-local SQLite, `PRAGMA foreign_keys=ON`, WAL, `synchronous=FULL`, immediate write transactions, schema `user_version=1`, audit records with no token material. Future migrations must be explicitly versioned; an unknown schema fails closed.
- Principal records contain `id`, bounded host-set label, monotonic `epoch`, `created_at`, optional `disabled_at`. Credential records contain `id`, `principal`, `salt`, `verifier`, creation/expiry/revocation timestamps. Audit records contain event, principal, optional credential and policy epoch.
- Issue, rotate, revoke and disable are **trusted-host-only** APIs. Rotation revokes the old credential, inserts the new record, increments the principal epoch and appends the audit event in **one** immediate SQLite transaction.
- A successful revoke/disable acknowledgment means a **fresh** authentication after commit will fail, including after process restart. It does **not** retroactively prevent an in-flight tool from writing. A future File Gate commit/lease fence (01d) must enforce that stronger guarantee.
- New principals have **no grants**. An authenticated agent is not an authorized note reader.

## Threat and deployment assumptions

- Protect the SQLite database directory with OS account ownership, permissions and backups controlled by the host. This module does not create the final app-data path or enforce Windows DACLs; the installation integration must do so.
- An untrusted local process that can read files cannot recover a high-entropy bearer from the verifier alone. A same-user process with full OS debug, file-write or memory access is **not** isolated by this stage.
- Database corruption, schema mismatch, CSPRNG failure, unknown/expired/revoked credentials and invalid token syntax never produce an authenticated identity. Host errors do not return token material.
- Do not back up plaintext credentials. Backups of the SQLite verifier store should be protected and restore tests must preserve revocation monotonicity; restoring an older database snapshot is a rollback threat that a host-maintained anti-rollback witness must address before a security claim about backup rollback.
- A verified principal must be bound to a **trusted stable VaultId** before any note or metadata becomes visible. A user-declared vault path is not permission evidence; neither is local Akula workspace identity automatically a globally shared VaultId.

## Mandatory next steps (not included in this slice)

| Work | Requirement | Release gate |
| --- | --- | --- |
| 01a next | Host-only API/UI issuance; private app-data location; explicit scoped-vs-legacy switch; verified principal in HTTP request context | Scoped mode fails closed; old global Bearer rejected with no fallback |
| 01b | Broker grant store for `AgentPrincipalId` and host-registered `VaultId`; reader tool router receives authorized context | Agent A can read X and cannot read Y, agent B can read Y and cannot read X |
| 01c | Per-hit filtering of search, workspace list, snippets, names, counts and history | No foreign resource metadata or existence leakage |
| 01d | Write permission, human approval, File Gate epoch/revoke commit fence | Revoke before commit prevents write, CAS regression stays green |
| 01e | Installed OS/UI acceptance and safe legacy migration | Native Windows/Linux, two official SDK clients with different Bearers |

**Operational rule:** do not enable opt-in agent-scoped HTTP authentication unless the authorized dispatcher is present and defaults to **no tools**. Authenticating a token, then passing its request to `tools::call(core, name, args)` without a vault grant would be a security bypass, not a milestone.

## Test evidence to obtain before this PR can be merged

1. Full Rust workspace + Sync on exact HEAD; native Windows MCP and all Main PR gate checks on exact HEAD.
2. Unique credentials for distinct principals; wrong principal/secret denied; persistent revoke after reopen; atomic rotate; disable after reopen; expiration; malformed tokens; corrupt or unknown schema; verifier file contains no raw bearer.
3. Preserve legacy production runtime byte-for-byte in behavior; existing official TypeScript MCP SDK direct read/auth/CAS must still succeed on the older shared-token mode when run.
4. **Do not claim** the two-Bearer different-vault isolation test until 01b has passed, and do not claim in-flight cancellation until 01d has passed.
