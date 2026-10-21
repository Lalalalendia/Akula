# AKULA-SERVER-BOOTSTRAP B1b2b0 — signed Node actually tested in an independent Ubuntu VM

**Status:** isolated experimental installation proof, not a production installer. The existing read-only Node D1 is intentionally bounded and not a persistent daemon. Parent plan: https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34

## What must pass on exact-head CI

1. Build the existing Node D1 lab and a small Rust example using the B1a verify_signed_release and VerifiedRelease::verify_package_stream APIs. The CI runner generates a fresh, disposable Ed25519 signing seed. Private signing material is **never copied to the guest**.
2. Sign real v1 Node executable and a deliberately nonworking, but correctly signed v2 candidate using the same ephemeral public key. Provision the *public key* independently through the guest's cloud-init seed before VM boot. Never learn this key from the candidate file.
3. Download Ubuntu 24.04 cloud image and authenticate its signed SHA256SUMS using apt-managed Ubuntu cloudimage GPG key. Verify image SHA-256 before booting a separate QEMU VM with its own Linux kernel. Run TCG if KVM is unavailable.
4. SSH to the isolated guest only with its strictly pinned pre-provisioned Ed25519 host key; reject password and direct SSH root authentication. QEMU user networking forbids guest egress. Upload signed artifact, B1a manifest and signature, and an independent B1a verifier. **Verify all bytes, exact size, expiration and Ed25519 signature inside the guest** against the pre-installed root-owned public key, both before and after copying into a root-owned release directory.
5. Launch a real guest systemd service using DynamicUser, StateDirectory, ProtectSystem=strict, ProtectHome, PrivateTmp, NoNewPrivileges, localhost-only network and an ephemeral bearer via LoadCredential. Prove authenticated health, synthetic document read and unauthorized 401.
6. Confirm that modifying a signed artifact causes B1a verification refusal. Try a validly signed v2 candidate with a nonworking executable; require health failure and restore v1. Then inject actual SIGKILL after atomic candidate activation and before health, invoke a separate recovery process, restart v1, and confirm the synthetic note survives.
7. Stop service, remove guest fixtures and discard VM snapshot. Do not touch real VPS, desktop Akula or any user vault.

## Security nonclaims

- **Signing is not operator approval.** This experiment does NOT implement B1b1b's real UI consent, B1b1c's externally anchored witness or a trusted monotonic installed-version floor. The signer is an ephemeral public **test key**, not a production release authority.
- A root-admin/hostile-root attacker, full VM rollback, dishonest clock, credential theft and malicious OS image are not solved by this test. A local journal and SHA-256 checksum cannot prove anti-rollback after full disk restore.
- This is **SIGKILL**, not a host reboot, fsync protocol powercut test or a proof that atomic symlink operations withstand all failures. It is not a reviewed path-safe installer suitable for arbitrary servers. A future OS-owned helper must enforce inode custody and verified authorization independently, without accepting arbitrary shell commands.
- The guest CI user has passwordless sudo inside the temporary VM. It is not a production delegation scheme.
- Node D1 stops after a bounded number of requests and supports a synthetic read-only note only; it is not an always-on P2P Sync node.

## Next gate

B1b2b1: replace CI-fixed root shell procedure with an OS-owned, least-authority install helper that binds independently confirmed owner approval, trusted host identity, signed release, and robust recoverable state. Test real reboot and file-system crash points in a disposable VM; only then connect a Windows installation wizard. No real Linux/Windows deployment in this PR.
