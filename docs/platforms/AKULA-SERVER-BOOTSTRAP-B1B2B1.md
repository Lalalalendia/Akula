# AKULA-SERVER-BOOTSTRAP B1b2b1 — schema-clean VM and real OS-reboot recovery

**Scope:** experimental extension of accepted B1b2b0 PR #473. Uses a completely disposable Ubuntu QEMU guest and the existing B1a test-only signed, bounded read-only Node D1. This is **not** a production installer, release signer, always-on Node or authorized installation on a user server.

## Risks addressed

1. The previous successful VM run had `cloud-init status: degraded done` with `cloud-config failed schema validation`, despite independently successful SSH/release key checks. Official cloud-init schema defines `ssh_genkeytypes` with `minItems: 1`: a field value `[]` violates the schema. Since this fixture already provides a pinned host key in `ssh_keys`, remove the invalid redundant field. The guest must now pass both `cloud-init status --wait` and `cloud-init schema --system --annotate`, with no ignored warning. The runner separately verifies the preboot release-key SHA-256 and exact pinned SSH host key.
2. The previous SIGKILL test recovered without rebooting the guest OS; `/run/` credentials disappear on restart. For this *test only*, store the one-run bearer as a root-owned 0600 file under `/etc`, feed it via systemd `LoadCredential`, and remove the uploaded user-owned copy. No production credential storage or rotation is claimed.
3. Inject a real `SIGKILL` after v2's atomic active-pointer switch while an explicit `in-progress` file exists. Force guest `systemctl reboot` before recovery. Verify that QEMU stayed alive, the Linux `/proc/sys/kernel/random/boot_id` **changed**, pinned SSH key still authenticates, no unconfirmed service started automatically, the pending file and v2 pointer persisted, and a different guest process rechecks the signed v1 executable before restoring v1. Confirm bearer-authenticated health, correct synthetic file, no anonymous reads, and clean teardown.

## Lab acceptance

- Dedicated B1b2b Ubuntu VM CI on the **exact new HEAD**, plus mandatory full Application checks, all successful before merge.
- Evidence markers: `B1B2B_VM_CLOUD_INIT_SCHEMA_PASS`, `B1B2B_VM_OS_BOOT_ID_CHANGED`, `B1B2B_VM_OS_REBOOT_RECOVERY_PASS`, final `B1B2B_PASS`. They must correspond to actual logged subprocess and network checks, not echoed unconditional success.
- No changes to production hosts, users' personal vaults, the installed Windows Akula, root credential stores outside the ephemeral guest, File Gate, P2P sync or MCP.
- If schema validation or pinned host-key verification fails, stop **before** uploading/executing signed package fixtures. If reboot identity does not change, report failure rather than treating successful SSH reconnection as a reboot.

## Limitations still open

- **OS reboot is not power failure.** Graceful `systemctl reboot` after process SIGKILL still allows the kernel to sync buffers. Crash consistency under abrupt power loss requires QEMU force-off and separate persistence witnesses.
- A signed release is not proof of operator consent. B1b1b/B1b1c0 are inert research modules; no protected OS-owned owner-grant CAS or privileged installer IPC exists yet.
- Hostile root, full VM snapshot rollback, untrusted clock and signer-key compromise cannot be solved with same-disk local ledgers. Hardware or independent server-side monotonic state would be needed for such claims.
- This uses an explicitly bounded read-only D1 binary, with a one-time synthetic Markdown fixture and test signing key. No production service support, persistent credential security ceremony, package distribution or multi-file synchronization is proven.

Source: [AKULA-SERVER-BOOTSTRAP-02 plan](https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34). Official schema: https://docs.cloud-init.io/en/latest/reference/modules.html and validation: https://docs.cloud-init.io/en/latest/howto/debug_user_data.html.
