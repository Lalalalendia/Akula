# AKULA-SERVER-BOOTSTRAP B0c — independent disposable Ubuntu VM admission gate

**Status: TEST INFRASTRUCTURE IN DRAFT, not a release.** Depends on [B0b PR #436](https://github.com/Lalalalendia/Akula/pull/436) (feature-gated real OpenSSH) and the already merged [B0a #430](https://github.com/Lalalalendia/Akula/pull/430). Requirement sources: [B0–B5 installation plan](https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34) and [AKULA-DEPLOY-01](https://app.notion.com/p/3fe32a84beec817cb9b7e7fa5a552334).

## Why B0c is needed

B0b CI proved real OpenSSH, including the positive and negative signed KEX, with a disposable `sshd` process **on the same GitHub runner**. That is valuable, but insufficient evidence about a separate machine: guest OS identity, SSH daemon lifecycle, VM networking, independent host filesystem, first-boot configuration and host-key pinning still need testing.

B0c adds a **separate Ubuntu 24.04 virtual machine**, rather than a Docker container or a second SSH daemon on the runner.

## Proof design

- Runs on a fresh GitHub-hosted `ubuntu-24.04` runner. Downloads the public Ubuntu Noble AMD64 cloud image over HTTPS, verifies the GPG signature of `SHA256SUMS.gpg` using the system-trusted Ubuntu **cloudimage** keyring distributed by the runner's package manager, and checks the SHA-256 digest of the exact image. This authenticates **only the Ubuntu VM image**, not an Akula Node release or installation package; B1 still requires its own trusted signature/anti-rollback proof.
- Creates a disposable NoCloud `cidata` seed ISO with a random per-run instance ID and ephemeral Ed25519 host key. The expected SSH host key is generated **before** guest boot, outside the guest, simulating operator possession of an independently verified host public key. This is not remote TOFU or a production key distribution flow.
- Creates a non-root, non-sudo `akulavm` guest account with its own one-run Ed25519 client authorization. Generates a random unusable-over-SSH password hash to avoid locked-account rejections; `ssh_pwauth: false` and the guest SSH server explicitly forbid passwords, keyboard-interactive authentication, root login, and forwarding.
- Starts QEMU with a separate guest Linux kernel, 2 GiB RAM, two vCPUs and an ephemeral snapshot disk expanded to 8 GiB. Uses KVM when available and software emulation otherwise. Guest outbound network access is restricted by QEMU; guest port 22 forwards **only** to an ephemeral host `127.0.0.1` listener. No public ingress and no customer's host is involved.
- Waits for a **strictly pinned OpenSSH** connection using the pre-generated host key. It never runs `ssh-keyscan` to establish trust.
- Executes B0b's six real SSH subprocess tests unmodified against the separately booted VM:
  1. Correct signed SSH KEX + authenticated fixed read-only Linux facts.
  2. Fake host key B against a real host key A fails before client authorization.
  3. Wrong offline SHA-256, malformed or unexpected key rejected before network.
  4. Wrong client private key fails after valid host KEX.
  5. Unsafe local identity/symlink/permissions refused.
  6. Host/user/package input checks cannot be bypassed via direct probe construction.
- The test suite admits a **`MissingPrivilege` refusal** from B0a preview for the intentionally non-sudo guest. It does not grant rights to compensate.
- Shell `trap` kills QEMU, deletes the snapshot, seed and all ephemeral keys, and prints only a bounded disposable guest boot log on failure. GitHub job timeout is capped at 55 minutes. Nothing is installed on a real server.
- B0c does not modify `aquilum-core` or File Gate. This is a separate test-only script and workflow, inheriting verified B0b behavior.

## Expected evidence and refusal

`scripts/ci/akula-b0c-ubuntu-vm.sh` and `.github/workflows/core-server-bootstrap-b0c-vm.yml` are an **additional acceptance gate**. Both local B0b and separate-VM B0c tests must succeed on their respective exact heads before B0 can be called experimentally verified outside the runner host. If the cloud image, checksum, KVM/TCG, cloud-init, SSH user or VM resources fail, mark this proof **unverified** rather than treating an empty job or skipped test as green.

**B0c does NOT mean B1 is ready:** there is no production VPS connection, host-issued NodeId, real user approval, package-signature verification, systemd installation, rollback, durable InstallId recovery, hardened remote mutation or device pairing. The next independent work is B1's separate authorization/journal and **signed native package verifier**, not a shell script that installs Akula from the preview plan.

References for the disposable VM topology: [cloud-init QEMU guide](https://docs.cloud-init.io/en/latest/howto/launch_qemu.html), [Ubuntu cloud image NoCloud instructions](https://ubuntu.com/docs/public-images/public-images-how-to/use-local-cloud-init-ds/), and [cloud-init SSH host key schema](https://docs.cloud-init.io/en/latest/reference/modules.html).
