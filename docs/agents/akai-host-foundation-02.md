# AKAI-HOST-FOUNDATION-02 — OS-protected marker and deny-only pipe in one service

**Depends on [Foundation PR #485](https://github.com/Lalalalendia/Akula/pull/485). Test fixture and standalone Broker only — no production activation.**

Previously `--service-custody` checked the pinned pre-existing protected marker and parent without opening a pipe; `--service-pipe` hosted the secure pipe without needing custody. Passing separate OS experiments did not prove these checks coexist for a single service lifetime.

The new explicit `--service-custody-pipe` first verifies an actual LocalSystem SCM process and opens both owner/DACL-checked non-reparse handles under `C:\ProgramData\AkulaHostBrokerResearch`. Only after success does that same process open its first-instance, nonremote, restricted named pipe and report RUNNING. The two pinned handles are held until the service stops. No fallback to the unchecked IPC mode on failed custody.

The real Windows test provisions a disposable service and ACL-protected marker, proves the missing marker refuses SCM RUNNING, verifies a live named pipe twice using SCM process PID, SYSTEM token and dedicated enabled service SID **without sending bytes**, then proves an ordinary Users write ACE prevents a *fresh* startup. A single OS pipe instance may temporarily be busy; the read-only client retries only after kernel `ERROR_PIPE_BUSY` via a bounded 2-second `WaitNamedPipeW`, rechecks the original SCM PID and still performs all live SID/process checks. Permanent contention remains a refusal. After fixture ACL repair, the joined startup and pipe validation succeed again. The existing custody-only fixture still passes. Cleanup refuses existing resources and removes only newly created test objects.

**Not a trusted vault registration.** The marker contains no host vault ID, local replica ID or grants. The Windows binary is unsigned and not installed in Akula. ACLs are examined before RUNNING, **not continuously rechecked during a live pipe session**. The pipe drops bytes and issues no permission. Independent OS-protected issuer, physical root registration, protected epochs, antirollback, signed installer, authenticated client commands and MCP disclosure fence are still required. No product code, P2P or File Gate write changes.

Next: one protected installer-owned registry binding opened physical vault root identity to canonical `plugins::VaultId` and a local replica identifier. Do not mint authority from MCP JSON, local user SQLite or matching IDs.

[AKAI Agent master plan](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de)
