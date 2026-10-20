# AKULA-SERVER-BOOTSTRAP B1b3a — kernel-authenticated helper IPC, signed preflight only

**Status: experimental Linux-only, feature-gated Rust code. No root daemon or production install/write authority yet.**

Parent spec: [AKULA-SERVER-BOOTSTRAP-02](https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34). Prerequisites already in main: B0 pinned SSH proof, B1a signed releases, B1b0 staging, B1b1b owner-signature prototype, B1b1c0 inert witness, B1b2b2 disposable VM hard-off proof.

## Distinct new boundary

Previous tests proved that a signed Node can be executed and recovered on a disposable Ubuntu VM. None authenticated the *Linux process connecting to a privileged installer* or implemented a narrow IPC grammar. B1b3a establishes this boundary separately from actions.

- A real connected Linux `AF_UNIX` socket is required. The preflight method reads **SO_PEERCRED from the kernel**, checks a nonzero peer process PID, the connecting UID and its own expected helper eUID BEFORE parsing data. These facts are never taken from socket JSON or caller arguments.
- Unlike peer-credential helpers that read a kernel PID directly into a `NonZeroI32`, this Linux-only code reads a plain C-layout ucred and explicitly rejects PID zero. This avoids a known concern in the rustix peer-credential type conversion. It is still a limited local IPC identity check, **not** remote Windows user consent.
- The only versioned opcode is **Preview=1**. The message has a strict byte length, fixed domain separator and schema, and only typed fields: `InstallId`, plan digest, SSH host fingerprint, authenticated manifest fingerprint, package SHA-256, release sequence, expected witness revision, nonce, issued/expiry timestamps and Ed25519 signature. No hostname string, filesystem path, arbitrary argv, shell, service name, URL, user-provided public key or write opcode.
- The Ed25519 owner-public-key pin and **exact** expected plan/host/package/floor values come from a separate `RootTrustedPolicy`. A caller's self-signed approval cannot supply that policy. One claim may be at most 15 minutes old and is rejected when not yet valid or expired; a valid signature for a different host, plan, release or witness generation is not accepted.
- Input is read with a bound, fixed-size frame and two-second read timeout. Client must half-close after exactly one frame; trailing/pipelined bytes, unknown opcode, malformed length or extra data fail closed. No grant is produced.
- Success returns only an **InertPreflightResult**, which always says `requires_independent_commit=true`. It never writes, stages, activates, creates users, runs systemd or grants privileges.

## Tests and evidence required

12 ordinary integration tests create **real Linux UnixListener/UnixStream** connections (not mocked peer identity) and test correct kernel UID, valid Ed25519 signature and scope; wrong process UID/policy eUID; wrong approver key; modified signature/body; independently signed but mismatched host/plan/floor; expired/future/overlong claims; bad version/opcode; truncated and pipelined extra bytes; zero nonce. Test seeds and identities are synthetic, never production keys. An additional ignored-by-default Linux CI test launches a **separate UID 0 process** through passwordless sudo on the ephemeral runner. It sends a valid signed request to a nonroot Unix socket helper and requires SO_PEERCRED to return WrongPeerIdentity; this is not a fake UID string in the payload. Dedicated Ubuntu CI runs this negative test explicitly, plus the 12 ordinary tests and no-feature Core regression. All must pass on exact HEAD.

## Critical nonclaims

1. This does **NOT** run as an OS-owned daemon yet: tests run under their normal CI UID with policies constructed in memory. A genuine root-owned helper, root-managed trust store, peer/group authorization, a safe socket path and independent service/process isolation are still missing.
2. The new signed IPC preflight has a distinct domain from B1b1b signed-owner approval; passing it does not imply the user reviewed an immutable plan or that the B1b1c witness has accepted an atomic CAS. No durable nonce consumption, trusted clock or policy-key rotation exists.
3. The parsed fingerprint fields are compared to **preconfigured values**, not yet to verified Linux file descriptors. No immutable package inode, B1a verification inside a live helper, real root-owned staging or permission to invoke systemd. A root administrator and full VM snapshot rollback remain outside the security claim.
4. Do not connect this preview gate to an installer directly. Future B1b3b must add a minimal OS-owned daemon with trust configuration protected from the client, real nonroot peer test, verified FD-bound B1a package, signed-owner authority and independent witness CAS before one narrowly enumerated install action can become available.
5. Existing bounded read-only D1 Node is NOT an always-on production service, and this PR changes none of its runtime code. Windows Akula, user vaults and real VPS remain unchanged.

Success must be reported as **real signed local IPC preflight**, never as a deployed installer.
