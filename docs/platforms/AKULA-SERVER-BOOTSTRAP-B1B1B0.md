# AKULA-SERVER-BOOTSTRAP B1b1b0 — private owner-floor journal lab

Status: Draft, experimental Rust library. Depends on signed owner approval B1b1a PR #456; this is a stacked branch. This is neither a production installer nor release authorization.

## Scope

- Open a pre-created private Linux directory by a pinned descriptor, require mode 0700 and current actor ownership, and deny symlinks and unsafe state-file metadata.
- Initialize explicitly from independently trusted SSH host key and Ed25519 operator-key digest. Missing/corrupt state never silently initializes itself.
- Persist a typed monotonically increasing floor (release sequence, version, signed manifest identity), last consumed InstallId and owner-approval fingerprint in a fixed binary format with BLAKE3 corruption detection. The local digest is NOT a MAC or signature against attackers who can write to the directory.
- On every attempted record, take an exclusive lock, load and validate the current floor, match the independently pinned host/key, reverify signed owner approval and B1a/B1b0 identities through B1b1a, reject replays and old or equivocated releases. Do not allow caller to override the stored floor.
- Write fixed-size state to an exclusively created pending file, fsync file and containing directory, atomically rename, then fsync containing directory. Never silently clean abandoned pending files. Expose explicit interruption seams for targeted negative tests; only a successful commit returns a receipt.
- Linux-only, opt-in; does not alter installed desktop, File Gate, Sync, network, and does not execute or install anything.

## Boundary

This work does NOT verify the actual staged file inode: B1b0 StageReceipt can be fabricated by an untrusted local caller. Never treat a journal record as authority to activate a package. Real installer helper must reopen, pin and rehash the staged inode inside the same protected transaction.

This module is owned only by the lab process (no real privileged/installer-owned permissions or TUF/TPM). Root or same-UID adversaries and physical restoration of older disk snapshots can roll back state. The supplied SSH host observation and trust anchors need independent authenticated provenance. There is no independently authenticated clock, operator key distribution, permanent one-time nonce service, installed service or hardware power-loss proof.

An injected failure just after rename but before directory fsync leaves power-cut durability uncertain: it is intentionally NOT a success. The lab exercises structure only. The next gate must test actual kill/powercut and trusted helper-controlled admission before systemd activation.

## Acceptance

- Exact-head Linux Rust workflow and default Core compile pass, followed by full Application checks.
- Successful initialization/reopen, monotonic higher release, negative replay/older release, wrong host/operator, tampering, unsafe directories/symlinks, and leftover pending-file refusal.
- Multi-handle/process concurrency and real interruption/power-loss are separate independent acceptance gates, not inferred from ordinary unit tests.

Parent plan: https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34.
