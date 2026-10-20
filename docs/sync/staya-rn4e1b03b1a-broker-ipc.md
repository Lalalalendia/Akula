# STY / Akula RN-4E1b-03b1a — first real Windows IPC broker process boundary (deny-only)

## Context and objective

The exact-head negative Windows OWNER/DACL preflight is accepted in [PR #434](https://github.com/Lalalalendia/Akula/pull/434) by [GitHub Actions #39349002364](https://github.com/Lalalalendia/Akula/actions/runs/39349002364), but **does not provide privileged custody**. [PR #433](https://github.com/Lalalalendia/Akula/pull/433) proved a counterfeit user-owned host SQLite can pass a structural read-only replica/grant check. We must stop ordinary Akula from becoming the authority issuing its own permissions.

This step creates an **independent binary crate** `aquilum-app/host-broker-lab` with no dependencies, no entrypoint from `native`, no modification to the main Akula Cargo workspace or lockfile, and no installer service registration.

## Implemented experiment: actual Win32 transport, ZERO positive permits

- Windows process checks the **real process TOKEN_USER** against the LocalSystem well-known SID before creating a named pipe. An ordinary desktop process cannot become the server merely by launching this binary.
- Creates one `\\.\pipe\AkulaHostBrokerResearch.v1` via `CreateNamedPipeW` with `FILE_FLAG_FIRST_PIPE_INSTANCE`, `PIPE_REJECT_REMOTE_CLIENTS`, explicit protected security descriptor, and message framing bounded to 32 bytes.
- Pipe ACL grants LocalSystem and Administrators control, but the interactive caller only `FILE_READ_DATA | FILE_WRITE_DATA | SYNCHRONIZE`. It deliberately does **not** grant `FILE_CREATE_PIPE_INSTANCE`; generic write rights would incorrectly include the overlapping append/create-instance bit.
- After `ReadFile`, invokes `ImpersonateNamedPipeClient`, verifies an OS thread token can be inspected, and always `RevertToSelf` before responding. A RevertToSelf failure aborts. No authenticated DeviceId or QUIC peer is inferred from a PID/SID alone.
- Protocol v1 fixed 32-byte request `AKHB|v1|op=HELLO|nonce16|reserved8`, response `AKHR|v1|deny-status|echo-nonce16|reserved8`. **All statuses are denials**: `Hold=1`, `Malformed=2`, `Unsupported=3`. There is no `Allow` value, signing key, enrollment or file mutation.
- Invalid length, future version, unsupported op, zero nonce and nonzero reserved bytes fail closed. Deterministic byte-mutation tests on Windows and Linux. CI compiles the actual Windows Win32 imports, but does **not** run a positive LocalSystem service.
- The binary accepts a single connection only; it does not persist state or implement SCM recovery. Nothing in Akula calls it.

## Remaining security work (blocking)

1. **Privileged installer/SCM lifecycle:** independent signed executable, service SID, controlled restart/upgrade, OS-protected storage for host policy, no appdata fallback, positive owner+DACL and service identity fixtures, adversarial same-user tests. No `runas` mock or test-only trust bypass.
2. **Client mutual authentication:** desktop opens only the intended local named pipe and independently validates server owner/Service SID using the live handle; resist a fake pipe pre-created under the same name. Refuse missing or unexpected owner. Server must authenticate client user/session and local registration (a SID alone does not authorize a device).
3. **Truth of the QUIC peer:** a client-provided `remote_id` field is just attacker-controlled bytes unless the trusted Broker itself owns the authenticated QUIC `Connection` or receives independently verifiable, non-replayable channel binding from another trusted component. Choose the trust ownership topology before implementing grant issuance.
4. **Durable broker-owned permissions:** explicit P2P-local-replica enrollment (not S3 profile), `VaultId`, pinned root/lock, selected `DeviceId`, trusted peer key, issuer, object hash/length and monotonic policy epoch, with transaction/CAS and refusal across restart. Same remote `VaultId` on two legitimate local replicas is allowed.
5. **Host/File Gate atomicity:** network retrieval must not retain SQLite writer locks. Future Code-owned, non-callback permit and File Gate must linearize grant revocation, OS custody, journal intent, no-replace, readback and ambiguity HOLD. Bao+G2-06 independently blocks production transfer. No `APPLIED` or ACK before durable evidence.

## CI and acceptance

[Windows named pipes and impersonation](https://learn.microsoft.com/en-us/windows/win32/ipc/impersonating-a-named-pipe-client), [CreateNamedPipeW / reject remote clients](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipew), [named-pipe access rights](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights). CI must pass Linux+Windows exact-head `--locked` standalone-crate tests, fixed-parser malformed/future-message tests and source isolation. **Even green CI will accept only this denial-only transport foundation.**

**Draft/HOLD; no installed product activation and no claimed working privileged Broker.**
