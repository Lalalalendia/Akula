# AKULA-SERVER-BOOTSTRAP B1b1c0 — inert witness ledger with signed CAS

**Status:** Linux-only cooperating-writer laboratory, **not** a privileged installer, OS-owned persistent authority or accepted anti-rollback mechanism. Source plan: [AKULA-SERVER-BOOTSTRAP-02](https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34). Stacked on [B1b1b #465](https://github.com/Lalalalendia/Akula/pull/465), with B1b1a #459, B1b0 #449 and B1a #448 accepted.

## Evidence and implementation

The module `server_witness_lab.rs`, compiled only with Linux feature `server-bootstrap-b1b1c-witness-lab`, is a restricted **local journal protocol experiment**. It accepts **the original signed owner approval**, a separately pinned approval public key, an independently supplied ExternalApprovalScope and original B1b0 StageReceipt and B1b1a ProposedFloor. It invokes B1b1b `verify_owner_approval` INSIDE the write method, rather than accepting caller-forged `InertWitnessCasRequest` as authority.

For each cooperating-writer accepted record it:

1. Opens a separately provisioned private `0700` directory, ownership-checked, with pinned Linux directory FD and leaf O_NOFOLLOW. All files are `0600`; no system service or executable path is ever touched.
2. Locks the directory with flock, reads each consecutive record and checks exact binary schema, checksum, previous tip and previous/released floor heads. Bounded at 512 generations. Rejects corrupt, truncated, symlinked, missing-intermediate and orphan pending entries. Refuses to clean an interrupted write implicitly.
3. Requires a **caller-supplied, externally pinned** `WitnessCheckpoint` (witness revision, floor head and previous witness chain tip), equal to observed state, so a changed or stale checkpoint yields CAS conflict. Genesis is **only** admitted if independently authorized by an outside provisioner; absence of a file is NEVER proof of new trusted enrollment.
4. Encodes the signed operator claim, InstallId, plan, SSH-host-key digest, staged artifact digest, previous/proposed floor, one-time nonce, previous witness tip and revision in a deterministic fixed-width binary record, checksummed using domain-separated BLAKE3. This is **not a secret MAC or signature** and does not resist a privileged hostile writer.
5. Creates `witness-<revision>.pending` with O_EXCL, synchronizes contents, renames to `witness-<revision>.record` with NOREPLACE and fsyncs the parent directory. Reopens and verifies the committed record before returning.
6. Tracks every nonce and InstallId from retained history and rejects reuse at later revisions. Exact retransmission after a lost ACK returns `AlreadyRecorded` with NO new grant and NO second write. A different nonce at the same revision returns CAS conflict.
7. Returns only `WitnessDecision::{NewlyRecorded,AlreadyRecorded}`; both say `requires_separate_activation_authority() == true`. It does not execute, install, activate systemd, provision NodeId or change permissions.

## Negative tests

12 synthetic Rust tests cover signed positive transaction; exact lost-ACK replay after reopening; tampered signature; wrong signer and pinned host; stale revision; consumed nonce across revisions; chained second release; corruption; pending-record crash hold; concurrent independent file handles; symlinks and unsafe directory mode; forged external checkpoint. A dedicated Linux workflow runs these plus B1b1b and B1b1a regressions and default-Core check. CI evidence must be checked on the exact merged-branch HEAD.

## Explicit gaps: not production

- **There is no actually independent or OS-protected witness.** The `WitnessCheckpoint` is only caller-supplied; the trust source, durable independent CAS, provisioning, rollback detection after full disk snapshot rollback, identity and trust-chain rotation do not exist. A malicious same-UID actor/root can rewrite/replace this same-directory journal; without a remote monotonic anchor or hardware-backed state, no local hash chain prevents malicious VM snapshot rollback.
- **No real consent ceremony.** Signed test approvals prove cryptographic possession of a reproducible synthetic private key, not that the user saw an accurate immutable plan and gave permission. The nonce only resists reuse in the particular intact local history.
- **No real kernel-pinned artifact authority.** This file-only interface trusts typed B1b0 receipt; it does not hold an immutable sealed Linux executable inode and cannot authorize its future execution.
- **No real crash/powercut proof.** fsync/rename is useful but crash injection must be verified with another OS process and reboot / VM disk failure separately.
- **No privilege separation.** Directory is only owned by the lab process, not independently protected root/OS helper. The protected helper and external monotonic witness must be a separately reviewed B1b1c1 gate before B1b2 systemd.

Next: run exact-head tests, fix any compiler/security errors, merge only after B1b1b is accepted and branch contains only B1b1c0 own changes. Then build an independent authority using an explicit privilege boundary, recovery/powercut tests and a falsifiable key/snapshot threat model. **Never equate accepted local ledger with approval to install.**