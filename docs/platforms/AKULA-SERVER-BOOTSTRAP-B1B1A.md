# AKULA-SERVER-BOOTSTRAP B1b1a — inert, independently anchored release-floor ledger

**Status: experimental, no installation authority.** Parent: [AKULA-SERVER-BOOTSTRAP-02](https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34). This experiment depends on already merged B1a signed manifest verification (#448) and B1b0 staged package journal (#449). There is no systemd mutation, package execution, root-owned helper, remote SSH transfer, NodeId or File Gate write in this PR.

## What is actually implemented

- Feature-gated Linux-only module `server-bootstrap-b1b1-floor-lab` combines a typed B1a `VerifiedRelease`, complete rehash of artifact bytes and exact B1b0 `StageReceipt`; the receipt alone cannot advance the floor.
- A **separate private ledger directory**, opened with O_NOFOLLOW, enforces 0700 directory ownership. Files are created at 0600 via directory-relative O_EXCL, fsync(file), renameat2(NOREPLACE), fsync(directory). Incomplete `floor-<generation>.pending` forces an explicit `NeedsRepair`; no implicit deletion/repair.
- The caller supplies an **independently trusted** `TrustedFloorHead` (generation, release sequence, version, manifest fingerprint and hash-chain tip). Genesis is only valid for an explicitly provisioned new ledger; absence of files is never treated as implicit genesis.
- The proposed new generation must strictly increase release sequence and must not reduce semantic version. Record bytes bind predecessor chain tip, current release identity, synthetic InstallId, plan digest, pinned SSH host-key digest, package length and SHA-256. Duplicate identical record is recognized on retried call without a second commit; changed host/plan/release on the same generation refuses.
- An append-only local record has a *non-secret* BLAKE3 checksum. It rejects accidental corruption and is checked against the independently supplied chain tip. It is **not** a MAC, signature or malicious-root-proof witness. Records are not used to authorize or launch anything.
- The output `ProposedFloor.requires_external_witness=true` is mandatory even after fsync: the current code **does not persist a trusted external approval/head**. A crash after rename can replay the same proposal. A crash before rename leaves the pending file and blocks future operations until a separate audited repair policy exists.

## Threats intentionally unsolved

1. **No real operator approval.** `LabPlanBinding` is typed consistency information, not proof of human consent or authorization CAS.
2. **No independently durable witness or trusted release floor.** The caller must bring its own protected head from an independently trusted source. A hostile owner/root, VM snapshot rollback, trusted-clock rollback or host reinstall can still defeat unanchored local state; an on-disk unkeyed hash does not fix that.
3. **No hostile same-UID/root adversary fence.** A private 0700 directory and flock serialize cooperating writers. O_NOFOLLOW prevents leaf symlink following; an attacker able to replace trusted parent directories or process memory remains out of scope.
4. **No B1b2 service activation or rollback.** No binary is executed, no package installed, no systemd service started/restarted or data schema migrated. File Gate, Windows Akula, VPS and personal data remain unchanged.
5. **No physical powercut durability guarantee.** fsync/rename is checked only on normal Linux runners; explicit separate-process SIGKILL + VM reboot + storage fault injection are required.
6. **No real production signing key ceremony, key revocation, TUF-like freshness, or secure time.** Only deterministic public test keys are used in regression tests.

## CI acceptance for B1b1a

`.github/workflows/core-server-bootstrap-b1b1-floor.yml` runs nine synthetic Linux tests for replay/reopen, multi-generation monotonicity, rollback, wrong host/InstallId, corrupt record or independently pinned head, orphan pending records, wrong package bytes, symlinks/directory modes and competing directory handles. It also checks ordinary Core without the experimental feature. This PR remains Draft until exact-head dedicated tests and required Application checks pass.

Next milestone: separate B1b1b **owner-authorized approval CAS + durable independent witness** with safe crash recovery, followed by B1b2 actual systemd sandbox installation only in disposable VM. Never interpret a local ledger proposal as install permission.
