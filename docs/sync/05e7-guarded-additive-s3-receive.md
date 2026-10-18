# SYNC-05e-7 — Guarded additive S3 receive

Status: opt-in, crate-private, single-file additive create. No scheduler or user-facing Sync UI activation.

The existing, separately tested S3 metadata pin, bounded S3 content transfer, WAL/FULL staging and external host-custody no-replace File Gate are now connected in one entrypoint. This is an actual new-file publication after host approval, not an overwriting sync executor.

Fail-closed gates: host registered root; SQLite cache and WAL journal outside the user vault before staging;  previously approved exact shared remote profile/VaultId; complete current S3 revision metadata; independently verified single Content tip and ancestor DAG; no tombstone, rename, sibling branches, stale selected revision, altered path, reserved internal directories or portable collision with any other remote document; no existing per-document baseline; bounded blob BLAKE3; unchanged mutable S3 head including ETag during staging. Native File Gate retains dirty editor checks, external OS lock, one-shot SQLite journal claim, pinned filesystem identities, no-replace publication and readback.

No code here invokes mark_applied, mark_verified, commit or S3 anchor advancement. The returned native creation observation is NOT a confirmed baseline. A crash or uncertain create is inspected, never retried blind.

Acceptance: pure conflict tests, real two-Core MinIO verified transfer and native create, Rust/core, documentation and Main PR gate at the exact final commit.

Remaining production work: native guarded replace-if-unchanged with durable original preservation, cross-OS receipt and crash recovery, then checkpoint promotion after independently verified local readback. Do not enable general automatic S3 synchronization or delete/GC on the strength of this additive-only step.
