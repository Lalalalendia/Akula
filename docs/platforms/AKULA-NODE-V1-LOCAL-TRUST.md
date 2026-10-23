# NODE-V1 — local trust and durable one-use authorization boundary

Status: **not a production Node release**. The code is on the single
`feature/akula-node-v1-integration` branch, not enabled by the ordinary
Akula desktop build. No public listener or server-install opcode is added.

## Separation of authorities

| Material | Holder | Meaning |
| --- | --- | --- |
| Signed Windows SSH KEX host pin | Windows Akula | Identity of the SSH installation host; not a Node data grant |
| Release manifest / root CAS receipt | root-only installer | Provenance of installed ELF; not service activation authority |
| Node Ed25519 private key | private non-root Node StateDirectory | Stable Node identity and signed **local-only** health challenge |
| Owner public key, exact Vault, roster/grant digests, epoch | root-owned /etc/akula-node/owner-policy.json | Independently provisioned read authorization pins |
| SQLite issued challenge ledger | non-root private StateDirectory | Atomic issued-first, single-use replay rejection |
| Current Iroh remote_id | actual authenticated QUIC connection | Cryptographic transport identity, not permission by itself |

The root policy is read using a pinned Linux directory handle and NOFOLLOW
child open. The owner signature on the exact read grant and the exact signed
roster are checked against independently root-owned pins. A changed policy
causes a **HOLD** until the process reloads and its session reauthorizes.
A peer cannot nominate its own owner key, epoch or protected Node identity.

The service identity file and SQLite state are checked for exact UID, 0600
files and 0700 StateDirectory, with no symlink/hardlink file admission.
Root policy files must be root-owned and non-writable by group/others.
The service refuses to run as UID 0. A locally trusted same-UID adversary
is not defeated; the installer must isolate this UID.

## Local binary

With explicitly selected Core feature `node-v1`, the separate Linux
`akula-node` binary recognizes exactly two modes:

- `--public-identity`: print only the non-secret Node Ed25519 public key.
  Used by a future privileged installer for independent policy enrollment.
- `--serve-local-health`: require fixed root-owned owner policy, open
  `/run/akula-node/health.sock` under a 0700 RuntimeDirectory and accept
  only a bounded one-byte-version plus 32-byte random challenge. The reply
  is a domain-separated Ed25519 proof from the stable key. A rejected
  request produces HOLD; there is no remote upload, Vault read, TCP, or
  Iroh listener.

**LOCAL_ONLY is not Healthy/Ready.** This unit proves only installed local
process identity when the caller independently verifies the signature and
fresh nonce. The actual signed root helper, service-unit installation,
systemd lifecycle, health-to-Windows bridge and cold-reboot inspection remain
pending and MUST NOT be inferred from this code.

## Durable challenge contract

- An issuer must first call `RootTrust::admit_read` with an actual signed
  roster, owner-signed exact grant and authenticated QUIC peer key.
- Only the resulting sealed Rust admission can create a fresh 256-bit
  challenge in SQLite. Issued rows bind peer, grant digest and root policy.
- Maximum 64 outstanding challenges; each expires after 60 seconds.
- Atomic SQLite DELETE with `BEGIN IMMEDIATE`, `synchronous=FULL` and an
  independently re-read root policy consume the challenge exactly once.
- Missing, duplicate, wrong peer/grant, expired, policy-rotated and
  clock-rollback inputs are refused. Corrupt SQLite never falls back to
  an in-memory permissive fence.
- `interop/akula-staya` feature `node-v1-runtime` implements the existing
  Iroh `OneUseChallengeFence` trait using this durable implementation.
  The original default-off laboratory remains unchanged.

This is a local SQLite durability boundary, **not** a signed end-to-end
receipt after power loss and not an independent revocation oracle if a
hostile root deliberately rolls back both the policy and disk.

## Evidence and release gaps

Code contains negative tests for persist/restart/replay, wrong peer/grant,
unsafe permissions, owner-policy replacement and symlink, owner signature,
current roster and revoke. **None is a verified CI PASS until run on the
exact integration SHA.**

Still blocked: trusted installer enrollment and signed release, actual
systemd RuntimeDirectory/StateDirectory integration, authenticated Iroh
server listener, owner-approved scoped Put, durable blob/revision receipt,
second Akula Get and File Gate create-new, revoke after reboot and full
Windows A → VPS → Windows B E2E. Do not enable the install button or
merge this release slice until all of those pass.
