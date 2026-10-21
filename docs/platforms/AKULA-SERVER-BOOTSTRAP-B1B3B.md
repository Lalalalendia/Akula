# AKULA-SERVER-BOOTSTRAP B1b3b — real root-owned Linux helper, preflight only

**Status: disposable Ubuntu CI proof only; NOT a production installer, and still NO privileged write command.**

Parent [AKULA-SERVER-BOOTSTRAP-02](https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34). Dependency: [B1b3a PR #482](https://github.com/Lalalalendia/Akula/pull/482), which already implements kernel Linux SO_PEERCRED and Ed25519 fixed-frame Preview with 13 real-socket tests.

## What this step adds

1. A feature-gated Linux binary `akula-helper-b1b3b-lab` launched as a **real effective UID 0** process by transient systemd on a disposable GitHub Actions Ubuntu runner. Its unit uses NoNewPrivileges, ProtectSystem=strict, ProtectHome, PrivateTmp, PrivateDevices, AF_UNIX only and an explicitly writable `/run/akula-b1b3b-lab`. No TCP listener and no shell dispatch.
2. The root-owned namespace has mode 0755 and a root-owned 0600 marker and 0600 fixed binary policy file. The helper opens the namespace with O_DIRECTORY|O_NOFOLLOW, checks root UID, exact mode and directory type, reads policy via pinned `openat` with O_NOFOLLOW, checks regular file, UID 0, mode 0600, link count 1 and **fixed** canonical wire length before/after reading. Its configured signer Ed25519 public key and PlanDigest, InstallId, host-key hash, manifest fingerprint, package hash, release sequence and witness revision are never supplied by the socket client.
3. CI generates a fresh synthetic signing seed **outside** the root trust directory and creates requests independently, then uses sudo to provision only the generated public-key policy. The seed is not given to the root helper. The unprivileged runner cannot modify the 0600 policy.
4. The helper binds a fixed Unix socket under that root-owned namespace and initially creates it private. The disposable CI harness grants socket transport to the allowed runner group; the helper independently checks OS SO_PEERCRED and pinned `allowed_peer_uid` before parsing signed frames.
5. Real nonroot request succeeds as **Preview only**; separately executed UID 0 client with the exact same valid Ed25519 packet is refused; changed signature, signed different host, trailing bytes and truncated bytes are all rejected. The response is a single Boolean status byte, not an installation capability. Both systemd journal and client assertions must verify refusals.
6. Cleanup stops the transient service and removes only the namespace and executable installed by this CI script. It refuses to overwrite preexisting system files. Real VPS, installed Windows Akula, personal Vaults and sync do not change.

## Strict boundary and missing production controls

- Root-owned systemd service and protected policy custody are **real OS behaviors**; the temporary policy, key ceremony, unit and binary are nevertheless **test fixtures**. No real owner's UI consent is checked. B1b3a uses a domain distinct from the B1b1b approval format.
- Successful Preview is not an activation grant or an independent CAS witness. Its nonce is not persisted, so replaying the same still-valid Preview can succeed; there is no installer write operation or systemd mutation handler.
- Root compromise, VM snapshot rollback, malicious root-owned policy setup and clock tampering are out of scope. The helper checks preconfigured package hashes but does not open and verify a kernel-pinned executable file descriptor. SO_PEERCRED is not remote Windows identity or authority.
- The root process handles only a bounded number of local requests, is neither a persistent installation service nor an always-on Akula Node. B1b3c must bind the *actual* B1a-signed package bytes and independently authorized CAS state inside the root-owned service, then add precisely one audited install action with crash durable rollback. Arbitrary shell commands and unrestricted paths remain forbidden.

## Acceptance gate

Dedicated GitHub workflow must show `B1B3B_ROOT_POLICY_UNWRITABLE_BY_PEER`, `B1B3B_REAL_ROOT_SYSTEMD_PROCESS_PASS`, `B1B3B_ROOT_SERVICE_REAL_UID_AND_SIGNED_PLAN_PASS`, `B1B3B_SYSTEMD_JOURNAL_PREVIEW_ONLY_PASS` and `B1B3B_PASS`, plus exact-head Application checks. The full root-owned paths and Linux UID tests must originate from an actual transient systemd process, not a mock object. No merge before all checks PASS and source review.
