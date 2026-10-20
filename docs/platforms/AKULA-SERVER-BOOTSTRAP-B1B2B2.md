# AKULA-SERVER-BOOTSTRAP B1b2b2 — QEMU host-process hard-off with a persisted guest disk

**Status: experimental GitHub Actions test. No production installer, no real server, no personal Vault.** Dependency: accepted [B1b2b1 PR #476](https://github.com/Lalalalendia/Akula/pull/476). Project record: https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34

## New falsifier

Previous B1b2b1 proved signed Node recovery after guest `systemctl reboot`. That path lets Linux flush filesystem caches and gracefully close the VM, so it does not exercise sudden disappearance of the entire guest OS.

This B1b2b2 step keeps the official GPG/SHA256-pinned Ubuntu image as a **copy owned only by a disposable CI temp directory (0700)**, and starts QEMU with `snapshot=off`. All guest writes go into this one disposable qcow2. `snapshot=on` is disallowed because it would discard the guest's changes when QEMU is killed, turning recovery into a test of a fresh system.

1. Run all existing B1b2b1 checks: exact preboot release-key fingerprint, strict SSH host-key pin, cloud-init schema clean, guest B1a Ed25519 verification before/after staging, constrained systemd, HTTP 401 on unauthorized read, valid signed v1 read, corrupted bytes denied, signed-bad-v2 rollback, SIGKILL of installer and guest OS reboot+recovery.
2. On the **same now-persistent guest qcow2**, demand that a fixed, root-owned guest lab script verifies the staged v2 package against B1a again, writes a `previous=0001 candidate=0002` pending file, syncs it, stops the service, atomically points `active` to the intentionally nonworking signed v2, syncs, and emits `B1B2B_VM_FORCEOFF_BARRIER_AT_V2`. It then intentionally stays alive in a loop.
3. The independent **host** observes the marker over pinned SSH while both the QEMU process and the remote installer are alive; it sends **SIGKILL to QEMU itself**, *not* to `systemctl` or only the installer, and requires child exit 137. This is a test of losing the entire guest kernel without graceful shutdown.
4. The host starts a new QEMU process on **the exact same qcow2** and the same independently pinned guest host key; require a new `/proc/sys/kernel/random/boot_id` and the original root-owned release public key fingerprint.
5. Before recovery, assert the old pending file and v2 pointer **still exist**, and the unconfirmed v2 Node has not auto-started under systemd. Invoke a new guest process to re-verify signed v1 against the pinned public key, restore the old pointer, restart systemd and verify bearer-authenticated health, correct synthetic read-only note, and anonymous HTTP 401. Require explicit `B1B2B_VM_HARDOFF_PERSISTENCE_PASS` and `B1B2B_VM_HARDOFF_SIGNED_RECOVERY_PASS`.
6. Stop the systemd test service, delete guest fixtures, terminate QEMU and **destroy the disposable qcow2, test keys and artifacts**. All steps fail CI on missing evidence; no live server is contacted.

## Safety boundaries

- **QEMU process SIGKILL is not a physical host power loss.** The host filesystem, page cache, storage controller and drive remain powered; a successful test does not prove durability under a host kernel crash, controller write-cache loss, write tearing or simultaneous disk corruption.
- The barrier occurs **after an explicit sync** of the pending marker and active symlink. This proves cold recovery at one already-synced switch boundary, not all intermediate crash windows. The next acceptance layer must include independent before-sync, after-write, during-rename and directory-fsync fault injection with durable OS-owned transaction records.
- No full VM snapshot-rollback or hostile-root protection: the signed same-disk journal is not a trusted monotonic external witness. No TPM claim.
- The authorized owner consent, independently pinned OS-owned protected CAS, privileged helper, trusted release-key lifecycle and immutable execution inode are not built. This test operates with a test-only Ed25519 signer and a guest-specific passwordless-sudo CI account. It MUST NOT be used as a general installation script.
- Existing `akula-node-lab` is a bounded read-only single-note fixture, not an always-on Node daemon or synchronization service.

## Acceptance

- New exact-head specialized QEMU workflow plus mandatory Application checks PASS.
- Real job logs contain markers `B1B2B_VM_FORCEOFF_BARRIER_AT_V2`, `B1B2B_VM_QEMU_FORCEOFF_CONFIRMED`, `B1B2B_VM_HARDOFF_BOOT_ID_CHANGED`, `B1B2B_VM_HARDOFF_PERSISTENCE_PASS`, `B1B2B_VM_HARDOFF_SIGNED_RECOVERY_PASS` with actual root/SSH/QEMU process evidence.
- Distinct merger security review and no changed production Node runtime, Windows Akula, Vault/File Gate or customer VPS.

**Next step after this experiment:** build a narrow OS-owned installer protocol and independently authorized, durable approval/CAS; do not expand shell-based installation to production by accident.
