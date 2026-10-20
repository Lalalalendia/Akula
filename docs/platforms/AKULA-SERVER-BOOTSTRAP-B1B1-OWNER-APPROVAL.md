# AKULA-SERVER-BOOTSTRAP B1b1-owner-approval — signed operator approval laboratory

**Status:** Experimental opt-in library only. Not an installer, server release, writable endpoint, activation authority, or claim of safe VPS deployment.

Depends on B1a release validation and B1b0 private signed-artifact staging (merged PR #448 and #449). Parent: https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34.

## Narrow scope

- Operator signs a canonical binary approval with a separately pinned Ed25519 public key (test key only).
- Approval binds operator identity, InstallId, independently observed SSH host fingerprint, trusted B0 plan digest, complete B1a signed-manifest fingerprint, package SHA-256, release sequence and a short 15-minute validity window.
- Verification matches the signed approval against an existing B1b0 stage receipt, a verified B1a release and a caller-provided current release floor. It rejects stale floor, changed epoch, key, host, plan, package, stage, time or identity.
- The output is an inert read-only witness. There is no command to install, run, overwrite, change permissions or advance the floor.

## Unproven boundary (must stay closed)

- No trusted installer-owned operator key distribution, key rotation/revocation, independently durable release floor, host-owned approval nonce register, single-use challenge or OS identity authority.
- B1b0 StageReceipt is an in-memory object and is constructible by a caller: comparison against a receipt is not proof of pinned kernel inode. Reopen and rehash the sealed package in a trusted helper before a future activation.
- This library accepts an externally supplied SSH host fingerprint and clock. Its caller must obtain them from trusted channels; B0b's real signed key exchange remains a separately reviewed experimental component.
- Same signed approval can be replayed inside the validity window until a durable one-time acceptance journal exists; never allow an irreversible action with this witness alone.
- File Gate, Sync, installed desktop, systemd and production network bindings are unchanged.

## Next gate after signing: durable approval consumption

Add a separate host-owned persistent approval state and monotonic release floor with atomic fsync and recovery tests. Bind an actual verified staged inode, InstallId, host identity and operator signature in one crash-recoverable transaction. Only after that can B1b2 exercise a sandboxed systemd installation on an independent disposable VM.
