# AKULA-SERVER-BOOTSTRAP B1b3c — root-held B1a signed ELF descriptor

Status: feature-gated, experimental Linux-only code and one-off CI systemd process. This is NOT a production installer.

Parent: https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34
Depends on accepted B1b3b PR #484: https://github.com/Lalalalendia/Akula/pull/484

## Boundary established

The B1b3b helper previously verified only the SHA-256 and manifest fingerprint declared in its root-owned policy and signed local operator preflight. B1b3c reads the ACTUAL candidate Node package from a fixed root-owned release directory and authenticates it using B1a before binding its AF_UNIX socket.

Release public key is independently provisioned as root-owned 0600 release.pub in the trusted namespace, outside the candidate directory. The caller never supplies that key, filename, package location, release sequence or an executable command. The release package, canonical B1a binary manifest and signature reside in a fixed root-owned 0700 directory and are single-link, regular 0400 root files. Directory and files are opened with Linux O_NOFOLLOW (package through openat relative to a pinned directory FD).

The helper verifies canonical manifest bytes, Ed25519 signature, validity interval, B1a rollback floor, exact length and SHA-256 of the actual opened descriptor. The B1a manifest fingerprint, sequence and package digest must match the independently root-owned approval policy. Device, inode, size, mtime and ctime are checked before/after streaming and the exact verified file handle is HELD until process exit. Every subsequent Preview checks descriptor metadata stability. On Linux, even renaming the original pathname changes the held inode's ctime, so the helper intentionally fails closed with ChangedInode; pinning file identity is not the same as authorizing pathname replacement. No package pathname is reopened for a subsequent operation.

CI signs the actual existing opt-in read-only Node ELF with a fresh TEST signer, strips debug symbols before signing and provisions the exact signed bytes. A different temporary Ed25519 operator signer signs the fixed Preview plan, with its manifest identity and package hash derived from B1a verified metadata.

Dedicated ephemeral Ubuntu CI invokes the real UID0 systemd helper and tests positive signed FD plus OS-verified peer UID, signed wrong host, tampered frame, truncation and trailing data. For each tampered, symlink, hardlink, or wrong-release-key candidate, the CI must launch a fresh uniquely named real transient UID0 systemd unit; require process exit code 2, a specific B1a signed-package refusal in that unit's journal, and the absence of socket binding. A missing transient unit or an absent socket alone is NOT evidence of a valid refusal. Thirteen root-owned descriptor tests additionally probe inode swap, mutation, bad mode, expiry and rollback.

## Claims expressly NOT made

This still has NO InstallSignedNode opcode or filesystem installation action and never executes the verified artifact. A valid Preview is not permission or an independent durable witness, and operator approval is still a synthetic test key with a separately provisioned trust policy. The approved release floor is ReleaseFloor::initial() in this lab; it is not external monotonic rollback protection.

Keeping an FD open and comparing metadata is not an immutable inode seal against a hostile root. Root-owned path setup and keys are ephemeral CI fixtures, not the production key ceremony. No real Windows operator consent, independently CAS-protected authorization ledger, permanently running service, production Linux install, noncooperating-root defense or end-to-end persistent Node is proved.

The next implementation gate is a narrowly privileged OS-owned install transaction that checks a true owner-approved plan, independent monotonic CAS and verified package custody at the exact commit point, then performs ONLY a fixed signed Node install and recoverable systemd activation.

All CI files are disposable and cannot modify a customer's VPS, actual Windows Akula, File Gate, or user vaults.

## CI evidence correction, 20 October 2026

The first profile CI had 12/13 Rust tests pass; one test wrongly assumed Linux rename never changes inode ctime and was corrected to expect a fail-closed ChangedInode. In the subsequent profile run, all 13 root tests passed and the live service admitted the verified signed FD; however, `systemctl start` on a stopped, unloaded `systemd-run` transient unit failed with `Unit not found`. Earlier negative cases in the same run were **false positives** because no fresh service was started at all. The fixed harness uses a distinct transient unit for EVERY negative case, verifies a real failed process and its specific B1a refusal from journald, then finally launches a restored positive unit. This is a correction of missing test coverage, not a relaxation of the signer or inode checks.
