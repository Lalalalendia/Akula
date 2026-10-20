# RN-4E1b-03b1c — real Windows SCM process lifecycle (research only)

**Source:** [standalone Windows broker #438](https://github.com/Lalalalendia/Akula/pull/438) and [read-only client service-origin #443](https://github.com/Lalalalendia/Akula/pull/443). New changes remain in `aquilum-app/host-broker-lab`, a fully separate Cargo workspace. No installed Akula application, Core, native, S3, P2P network, File Gate, parent `Cargo.lock`, or user data are changed.

## Verified question

The earlier #443 client could inspect SCM, a named pipe PID and a specific Service SID, but **no real service existed**. Merely inventing a service SID or a mock LocalSystem token cannot answer whether an actual managed service can start, preserve its identity, stop cleanly and deny unauthorized IPC.

This implementation provides a real Win32 `StartServiceCtrlDispatcherW`, `RegisterServiceCtrlHandlerExW` and `SetServiceStatus` path behind the standalone research CLI's **--service** flag. It reports `START_PENDING → RUNNING → STOP_PENDING → STOPPED`, signals a kernel event in the SCM STOP/SHUTDOWN handler and closes it on shutdown. The service has **no pipe, no host DB and no command for granting access**; it cannot publish files or receive peer data.

The earlier client attestation is reused for a separate positive **--verify-scm** read-only check. It resolves the fixed service via SCM, pins the running process, checks the real SYSTEM process token and enabled non-deny-only service SID `NT SERVICE\AkulaHostBrokerResearch`, then requires the same SCM PID again. No handles/capabilities escape. The stronger **--verify-pipe** must STILL fail because the service does not own a pipe.

## Disposable Windows runner acceptance

The new Windows-only workflow deliberately tests the **real SCM** in a fresh GitHub-hosted Windows runner, never the user's PC:

1. Reject if a service with the exact lab name already exists (never overwrite).
2. Build the separate executable and verify `--verify-scm` refuses while no service exists.
3. Register a **manual-start own-process** service under LocalSystem using Windows SCM, explicitly configure `sidtype unrestricted` before starting it, and refuse if registration fails. No autostart, no installer persistence or changes to desktop Akula.
4. Start, require RUNNING and independently inspect the SCM account and nonzero PID. Invoke the real `--verify-scm` executable, requiring success against the actual SYSTEM/service token.
5. Invoke `--verify-pipe` and require **failure**, to avoid treating the installed service's mere existence as a usable IPC authorization endpoint.
6. Stop through SCM, await STOPPED and delete in a `finally` cleanup even if validation fails. Strict scope and unchanged Cargo.lock are enforced. Failure of the cleanup also fails the CI.

The existing Linux/Windows test matrix from #438/#443 still runs to detect protocol, Windows ABI and formatting regressions.

## Production gates that remain unresolved

A positive SCM fixture is NOT proof of a signed production service, trusted application executable/installer, Broker-owned protected database, WAL/SHM ACL, startup recovery, IPC client authentication, or production service upgrade/downgrade. The fixture uses a normal CI-built binary without code signing, in an ephemeral elevated testing environment. The research service still does not host the pipe, so it cannot issue permissions.

Next **RN-4E1b-03b1d**: support cancellable real named-pipe accept/read loops under the SCM service lifecycle, with Windows Service SID-protected IPC, bidirectional live-process/service attestation, explicit peer principal policy and no positive grant commands. Test normal connection/disconnect, client death, server stop while blocked, service restart and fake pre-created pipe. Enforce bounded cancellation; never signal RUNNING as P2P-ready until the pipe is created and pinned.

Next **RN-4E1b-03c**: privileged, installer-attested host policy store and distinct P2P-replica enrollment, physical root, DeviceId, actual authenticated QUIC EndpointId and monotonic policy/revocation epochs. Plain user-origin `remote_id` over IPC is NOT connection proof. Signed Bao bytes, STY-G2-06 and crash-safe File Gate publication remain separate P0 gates.

No `APPLIED`, receipt, ACK, user-vault writes or installed P2P sync. Keep this branch Draft/HOLD.
