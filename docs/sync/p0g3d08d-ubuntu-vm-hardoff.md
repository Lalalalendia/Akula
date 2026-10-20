# SYNC-P0g-3d-08d-2a — Linux QEMU hard-off at the real V8 seam

**Status:** Draft, experimental; exact-head CI not accepted yet. No S3 apply, remote ACK, native durable receipt, baseline promotion, or Windows coverage.

## Real test boundary

The host builds the **real Akula Core libtest** with `--features sync-s3` from the PR source. An independent Ubuntu 24.04 QEMU VM runs `p0g3d08c_writer_abort_entrypoint` with a deliberately test-only `AKULA_P0G3D08D_VM_PAUSE=1` variable. The native fixture executes the existing held-custody File Gate publication, strict external HistoryService version and SQLite V7/V8 candidate. The installed Akula executable and production Core paths do **not** acquire a new pause or publisher.

1. Fetch Ubuntu Noble cloud image after validating its Ubuntu-signed SHA256SUMS manifest with the system Ubuntu keyring. Record the signed image SHA in the job output. The image is not a signed Akula release artifact.
2. Prepare a disposable host `mktemp` workspace, signed-guest boot disk with `snapshot=on`, and a separate **raw ext4 guest-only data disk**, cached via QEMU `cache=none`. The host never mounts or writes a user vault.
3. Boot VM #1. Allow SSH only over QEMU's 127.0.0.1 loopback port forward, with generated pinned Ed25519 host key and explicitly trusted client key. The cloud guest gets no external networking via `restrict=on`.
4. Mount the disposable test disk as `/mnt/akula-hardoff` and copy the just-built test binary onto it. Before the destructive step, the test worker completes the protected original and baseline fixture and runs a Linux `sync` setup barrier.
5. The guest prints the exact fixture identity over **live SSH stdout** (host-owned log) and then performs V7, the real File Gate primary publication, immutable HistoryService and V8 SQLite insert. It emits `AKULA_P0G3D08D_VM_READY=after-v8` and parks forever without exiting or calling `abort`.
6. **Only the host** kills the QEMU process using SIGKILL after receiving that out-of-band V8 signal. This discards the guest kernel's volatile process and page-cache state; no guest shutdown/sync is invoked after the seam.
7. Boot a new guest kernel from the same ephemeral cloud image, remount the **same** ext4 test volume (normal filesystem journal recovery), and launch a fresh independent Core/HistoryService read-only verifier. Recover the root, app-data and operation ID from the *host's out-of-band transcript*, not an assumed durable guest marker.
8. The cold verifier requires the original bounded SQLite BLOB, previous confirmed baseline and non-replayable V7 attempt; it accepts only `LocallyConsistentHold` or explicit refusal. The operation remains `Staged`, and existing guards prevent `mark_applied`/ACK. Missing/changed original or lost one-shot fence is a test failure, **not** evidence that the drive is safe.
9. The job prints the cold classification and `AKULA_P0G3D08D_VM_REAL_FORCE_OFF_COLD_REOPEN_PASS` only after the independent guest test actually ran. On error, host-tail logs identify the failure. All scratch resources are deleted on exit.

## Why this is stronger than #444, but still not a durable receipt

#444 terminated **only the writer process** under a running OS. This stage actually terminates the whole **Linux guest kernel/VM**, reboots a new guest and reopens the same physical guest-visible ext4 volume from independent Core state.

QEMU SIGKILL is **not** removal of electrical power from the host SSD/controller. Its host page cache, drive caches, hypervisor flush semantics, write coalescing and real NTFS behaviour are outside this test. `cache=none` bounds one layer of caching but does not transform a VM test into power-cut evidence. A green result cannot authorize native receipt issuance.

## Acceptance and remaining steps

- Exact-head Ubuntu VM workflow completes real guest forced-off test and prints the out-of-band seam plus a cold HOLD/DENY classification, preserving the protected original, prior baseline and V7.
- Existing Windows/Linux Reserved S3, required Rust and Main PR gate remain green for the same test-only code.
- No simulated `std::process::abort()` is counted as a VM hard-off. No fake reporter/host-only fixture is substituted for the native Akula writer.
- No new production S3 writer, additional File Gate publisher, automatic retries or remote ACK.
- **Next:** extend actual VM power-off to V7, post-primary and post-history seams; include filesystem ancestor creation and independent app-data/vault disks. Develop a **separate** Windows/NTFS VM test with native path/FlushFileBuffers scrutiny; a Linux success must not be copied to Windows.
- **Then:** physical power-fault/storage tests and a separately reviewed authoritative native receipt protocol. Fail closed on unproved namespace durability.

Run manually on a disposable non-root Ubuntu runner with needed QEMU tools:
```bash
bash scripts/ci/akula-p0g3d08d-vm-hardoff.sh
```
The CI job runs this exact script on Ubuntu 24.04; no user machine or production credentials are involved.
