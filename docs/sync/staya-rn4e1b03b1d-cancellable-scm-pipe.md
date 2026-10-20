# RN-4E1b-03b1d — cancellable SCM-owned named-pipe accept/read (LAB / no grants)

**Dependency:** [RN-4E1b-03b1c PR #447](https://github.com/Lalalalendia/Akula/pull/447) (research-only actual Windows SCM lifecycle), from `afc25826b30afa56b484344ec2f86d9600629ca6`. The earlier exact-head service proof on `76157f56` passed [Windows CI #39381843361](https://github.com/Lalalalendia/Akula/actions/runs/39381843361), but a newer compiler/FFI-normalized #447 HEAD was still queued at this step.

## Why

A synchronous `ConnectNamedPipe` or silent-client `ReadFile` can block indefinitely, preventing SCM STOP/SHUTDOWN and keeping a retired broker instance alive. Merely naming a service and having it report `RUNNING` is not sufficient to control its lifetime. No P2P rights may survive a dead broker.

This slice adds a **separate opt-in** `--service-pipe` mode to the standalone `aquilum-app/host-broker-lab` (NOT an installed Akula feature).

## Implemented denial-only lifetime

- Reuse the exact existing `LocalSystem` process-token check and least-privilege SDDL from `windows_server::create_secure_pipe` for the dedicated `\\.\pipe\AkulaHostBrokerResearch.v1`. A client never receives `FILE_CREATE_PIPE_INSTANCE` rights; server uses `FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED` and refuses remote clients. No extra host policy store is provisioned.
- Prepare the protected pipe and manual-reset I/O event before reporting `SERVICE_RUNNING`. If the first-instance name is occupied, service refuses rather than falling back to a random name, mutable namespace or user-owned server.
- On each connect, use `ConnectNamedPipe` with real `OVERLAPPED` state and a separate event. Wait on **both** the SCM stop event and the pending I/O event with `WaitForMultipleObjects`. Cover the immediate client-before-connect `ERROR_PIPE_CONNECTED` race.
- On connection, issue only an overlapped, bounded one-byte `ReadFile`. All input is discarded, **no response, no permission, no File Gate**. A silent client cannot prevent STOP: SCM stop wakes the event wait, calls `CancelIoEx`, and **drains the cancellation with `GetOverlappedResult(wait=true)` before the OVERLAPPED/buffer is freed**. After return disconnect, and reuse the same pipe instance for a later client.
- Reset the event/OVERLAPPED between sequential operations; no simultaneous read/write on the same pipe instance. Oversized frames, broken clients and disconnects have no policy effect. Loop pacing is interruptible by the same stop event.
- Default `--service` behavior from #447 remains intact (SCM-only with no pipe). No native, Core, S3, QUIC, Bao, grant ledger, user file or root file is changed.

Microsoft reference material: [ConnectNamedPipe overlapped](https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-connectnamedpipe), [CancelIoEx cleanup](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-cancelioex), and [named pipe overlapped I/O](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-server-using-overlapped-i-o).

## Exact-head acceptance

The Windows GitHub Actions fixture installs **only into its ephemeral CI runner** a temporary `LocalSystem`, manual-start service with a dedicated Service SID; never register a service in the user's Akula installer. It must verify:

1. Real SCM PID/SYSTEM/SID and open-pipe PID/service SID agreement **while the service is running**, with no message sent.
2. SCM stop/restart when no client is holding the pipe (pending asynchronous connection).
3. SCM stop while a real named-pipe client is connected but **sends no bytes**, proving pending read can be cancelled; no indefinite Stop-Service.
4. After STOP, pinned pipe verification refuses. A new SCM START recreates its protected first-instance endpoint. All service registrations are stopped/deleted even when an assertion fails.
5. Windows rustfmt, standalone `--locked` tests and all original #438/#443/#447 regression gates; no changed main workspace Cargo.lock and no application scope.

**The test has NOT established a trusted signed installer, service upgrade/rollback, DOS resistance under many simultaneous clients, a durable server-owned P2P registry, actual QUIC channel binding, mutual authenticated application command API, or Windows shutdown behavior under power loss.** Even when the service pipe is live, the client's positive service-identity check is only source provenance, not authorization.

## Next P0

RN-4E1b-03c: create a genuinely protected Broker-owned registration/epoch store with real installer authority and narrowly scoped IPC commands. Prove `VaultId` shared across multiple legitimate physical replicas while permissions are bound to the exact replica, `DeviceId`, verified QUIC EndpointId, immutable issuer/hash/length and monotonically increasing revocation epoch. Until trusted connection ownership is designed, user-supplied `remote_id` over IPC must be considered attacker-controlled. Publish verified bytes only via future Core File Gate durable no-replace path, gated by STY-G2-06. Signed receipts/`APPLIED` remain HOLD.

Keep Draft. Never install or enable research broker on the user's PC.
