# RN-4E1b-03b1b — read-only Windows named-pipe service origin verification

**Status: independent research only / Draft / product HOLD.**

This slice is stacked on [RN-4E1b-03b1a PR #438](https://github.com/Lalalalendia/Akula/pull/438) and uses its separate `aquilum-app/host-broker-lab` Cargo workspace. The original implementation can only return HOLD/MALFORMED/UNSUPPORTED. No Akula desktop, Core, P2P File Gate, installer or sync code imports this crate.

## Threat to falsify

An ordinary local process can create a syntactically valid named pipe under the expected name. Its messages cannot establish host-installed P2P authority. In particular, a PID found by `GetNamedPipeServerProcessId` alone must not become an authentication result; a name, PID or LocalSystem SID by itself is insufficient evidence of the expected *installed service*.

The client-side `windows_client_attestation::inspect_installed_service_pipe_without_sending` probes a **fixed** lab service name `AkulaHostBrokerResearch` and one named pipe instance without sending any request bytes.

## Sequential fail-closed observations

1. Ask the Windows Service Control Manager to open exactly `AkulaHostBrokerResearch` with `SERVICE_QUERY_STATUS` only. Require `SERVICE_RUNNING`, dedicated own-process service type and a nonzero registered PID. Failure denies without even opening the IPC endpoint.
2. Open the named pipe with **FILE_READ_DATA only** (not `GENERIC_WRITE` or `FILE_CREATE_PIPE_INSTANCE`), using `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION` so a malicious pipe server cannot gain full impersonation through a client handshake.
3. With the pipe handle pinned, query its server PID. It must match SCM. Pin that process with `PROCESS_QUERY_LIMITED_INFORMATION` and read its OS process token; its user SID must be LocalSystem.
4. Resolve the specific Windows service SID `NT SERVICE\AkulaHostBrokerResearch` through the OS, and require it to be **present, enabled and not deny-only** in that pinned server process's token groups. A plain LocalSystem process with no proper service SID must be refused.
5. Re-check both the pinned pipe server PID and SCM service PID before closing the handles. PID mismatch, missing service, missing token, changed process/service or any denied call returns an error. No command is transmitted and **no handle or write capability is returned**.

These checks form **observations**, not one atomic installed-host attestation. The SCM registration, service security descriptor, executable signing/configuration, privilege boundaries, protected host registry/WAL and service lifecycle have **not been established**. An administrator capable of changing SCM or privileged files remains within the trusted computing base. The Windows CI runner is not a positive installed-service fixture.

## Negative-only tests and bounds

- Real user-owned fake named-pipe server with matching process PID is not sufficient to prove SYSTEM or an installed service; a wrong PID is explicitly rejected.
- No SCM-installed service means a fail-closed result before requesting any application message.
- The previously verified denial-only parser tests and real Windows server FFI compile still run alongside this module on both Linux and Windows.
- This test module never calls `WriteFile`; the actual Akula native app has **no client connector**. No service install/registration, user file writes, grants, network access or policy mutations occur.

The source follows the documented OS APIs:
- [SCM process status](https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-queryservicestatusex)
- [Named-pipe server PID](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getnamedpipeserverprocessid)
- [Service SID types](https://learn.microsoft.com/en-us/windows/win32/api/winsvc/ns-winsvc-service_sid_info)
- [LookupAccountNameW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-lookupaccountnamew)

## Next hard gates: RN-4E1b-03b1c / 03c

- Actually signed and installed, independently versioned SCM service with `SERVICE_SID_TYPE_UNRESTRICTED` (or reviewed restricted policy), service SID-only protected policy directory and SQLite/WAL/SHM, upgrade/downgrade/stop/crash behavior, installer-run positive SYSTEM test and hostile same-user test. No manual `runas` substitute.
- Mutually authenticated IPC clients; a fixed service SID alone is not authorization for a DeviceId or a local replica. Introduce an auditable trust owner for authenticated QUIC `Connection::remote_id()` and a cryptographically bound grant epoch (do not accept client-supplied endpoint bytes as an authoritative identity).
- Broker-owned durable P2P replica enrollment (multiple distinct local replicas legitimately share one remote VaultId), revoke CAS, cancellation/race tests, and bounded File Gate guarded no-replace publication after verified Bao bytes and STY-G2-06.
- No `APPLIED`, receipt, ACK or user-installed autosync until the full chain is proved.

**A green Linux+Windows CI at this stage accepts only a read-only Windows service-origin diagnostic.**
