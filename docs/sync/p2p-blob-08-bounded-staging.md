# P2P-BLOB-08 — physically bounded staging volume (Linux lab)

**Status: research-only, stacked on [P2P-BLOB-07 PR #321](https://github.com/Lalalalendia/Akula/pull/321).** No changes to Akula main, desktop, File Gate, S3 V1/V2, protected workspaces, or Roha/noq forks.

## Why a reservation journal is not a physical quota

P2P-BLOB-06 and -07 limit active and durable *declared-byte* reservations. Those counters do not bound actual FsStore/Bao/outboard/redb database overhead, file count, write amplification, temporary files or untracked local writers.

Linux tmpfs supports a size limit but loses content upon unmount, so it cannot prove disk persistence after a remount. Instead this isolated laboratory formats **one brand-new 64MiB regular file image** as ext4 and mounts it through Linux loopback. The kernel enforces finite blocks for all data within that filesystem, regardless of the in-memory ledger.

References: [Linux loop devices](https://man7.org/linux/man-pages/man4/loop.4.html), [util-linux mount](https://man7.org/linux/man-pages/man8/mount.8.html), [tmpfs lifetime and size](https://man7.org/linux/man-pages/man5/tmpfs.5.html).

## Code and CI

- A new Linux-only Rust module, staya/roha-transport/src/bounded_staging.rs, implements a **mount-present preflight**, not a physical quota. It refuses symlink, unsafe/world-accessible or same-filesystem staging roots; pins a directory fd and can re-check inode/device to detect accidental missing/replaced mounts.
- The shell harness staya/roha-transport/lab/isolated-ext4-capacity.sh is restricted to a disposable Linux GitHub runner and its RUNNER_TEMP. It makes exactly one new regular 64MiB image, runs mkfs.ext4 on that private file only, mounts with loop/nosuid/nodev/noexec, and verifies both ext4 mount identity and statvfs reported capacity.
- A separately invoked ignored Rust test creates an **actual patched iroh-blobs 0.103.1 FsStore** in this finite image, writes and rereads a known BLAKE3 blob, and shuts down cleanly.
- A real, non-sparse bounded write stream adds additional staging bytes **on the same image as the real FsStore**, and must receive ENOSPC before reaching 64MiB. An already saved sentinel remains readable with its exact SHA256.
- The harness removes only its own filler, syncs, unmounts and remounts the same image. It checks persisted sentinel SHA256 and reruns Rust against the same real FsStore, verifying the saved blob is BLAKE3-exact after remount. After unmounting it tests the missing-mount refusal against the underlying host directory.
- A new dedicated read-only GitHub Actions workflow (.github/workflows/staya-p2p-bounded-ext4.yml) pins Rust 1.91 and Roha/noq SHAs, validates a locked dependency graph, and treats missing loop/mount support as a **failed** gate rather than a skipped success. It always attempts cleanup of its ephemeral mount/image.

## Required evidence

The exact-head CI must prove: (1) actual Linux ext4 volume ENOSPC at a finite capacity shared by real FsStore and other data; (2) clean remount retains a verified FsStore blob; (3) missing mount is rejected; (4) previous Roha/noq and P2P/SIGKILL security tests still pass. Until these tests succeed, this is an unaccepted experiment.

## Non-goals, limitations and next steps

**Not a production disk quota.** The Rust gate detects a different device but does not authenticate an explicit maximum size or prevent privileged concurrent remounts, path renames, nested filesystem mounts or direct writers bypassing the gate. Production needs fd-relative custody/sandboxing and independently verified OS-backed capacity.

**Not yet full startup reconciliation.** All real FsStore/Bao/DB files are not yet enumerated and tied to the P2P-BLOB-07 durable reservations. Unknown/orphan paths or metadata inconsistency must cause HOLD, not cleanup or silent reset. Current remount testing follows a clean FsStore shutdown, not an interrupted on-disk write or power loss.

No signed VaultId/RevisionId/DeviceId-to-EndpointId grants, Windows/Android adapter, File Gate, user document mutation or two-machine P2P transfer are enabled. Keep this stacked PR **draft** even after green CI. Next gate: read-only fail-closed startup reconciliation and interrupted physical writes on the bounded image; then physical LAN/relay experiments.
