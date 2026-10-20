# RN-4E1b-03c1a — Windows Broker custody provenance; no P2P grants

**Draft / Research only / Product HOLD.** Based on [RN-4E1b-03c0 PR #453](https://github.com/Lalalalendia/Akula/pull/453), which passed exact-head Linux+Windows [CI #39398038942](https://github.com/Lalalalendia/Akula/actions/runs/39398038942) on `80cc7766`. The separate host-broker-lab Rust workspace remains disconnected from installed Akula Core/native and File Gate.

## Implemented

The standalone research binary now has a `--verify-custody` read-only diagnostic and a distinct `--service-custody` real Windows SCM mode. The service cannot report `RUNNING` until the **pre-existing**, non-appdata Windows authority marker `C:\ProgramData\AkulaHostBrokerResearch\authority.marker` passes an OS protection check. It never creates, repairs or overwrites a missing marker.

Win32 `CreateFileW` pins both the directory and marker, with `FILE_FLAG_OPEN_REPARSE_POINT` plus directory `FILE_FLAG_BACKUP_SEMANTICS`, and rejects unexpected file identities, symlink/reparse targets and hardlinked marker files. Neither handle permits sharing deletion while the service holds it.

Native `GetSecurityInfo` audits the actual opened HANDLE, not the path's JSON content. Both the directory and marker require SYSTEM/BUILTIN Administrators as owner, an explicitly protected non-null DACL, a bounded list of known ACEs, **no namespace-write ACE for ordinary users**, and write ACEs for SYSTEM plus the exact OS-resolved `NT SERVICE\AkulaHostBrokerResearch` SID. Failure blocks readiness; the diagnostic returns no handle or capability.

The marker deliberately contains **no device, grant, vault, revocation epoch or peer metadata**. Its presence means only that this particular installation namespace passed the limited ACL check, not that any resource is authorized.

## Real ephemeral CI

The independent Windows CI test registers a disposable manual-start LocalSystem service with a dedicated service SID, proves startup refuses when the marker is missing, creates the marker and protected DACL as an installer-like privileged test fixture, and requires SCM `RUNNING`, SYSTEM/service token, and successful custody diagnostics. The full IPC origin check must still refuse because this service mode never opens the pipe.

The test then grants BUILTIN Users write access to the marker and must observe a fail-closed diagnostic and failed service restart. After the privileged fixture repairs the ACL, a fresh SCM startup is checked and all fixture service/dir objects are removed in a `finally` cleanup. The test refuses to replace preexisting system objects. A submitted PR is not a passed acceptance; both OS test jobs plus this real SCM fixture must succeed at one exact HEAD.

API references: [GetSecurityInfo](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-getsecurityinfo), [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew), [icacls](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/icacls).

## Unproven and release blocking

This is **not a durable policy database**. No actual revocation journal, SQLite WAL/SHM sidecar fencing, crash/power-loss recovery, rollback detection, or protected installer signing is established. The security of the full ProgramData ancestor chain, privileged hostile administrators, and dynamic ACL changes during RUNNING remain outside this test. LocalSystem and Administrator permissions are part of the trusted computing base.

No authenticated QUIC `Connection::remote_id()` origin, IPC grant issue, P2P enrollment, Core File Gate publication, signed receipt, `APPLIED` or ACK exists. All test code is outside installed Akula. The positive marker is **never** used as a remote device permit.

**Next RN-4E1b-03c1b:** independently prove a single protected Broker-owned durable journal or SQLite with protected `-wal/-shm`, monotonic epoch CAS in two OS processes, cold reopen, tamper and crash/HOLD recovery. No automatic empty replacement or sidecar alias. **Then 03c2:** actual QUIC connection ownership and channel binding, rather than client-provided EndpointKey bytes.
