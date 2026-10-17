# ADR-SYNC-007 — Cross-vault bilateral recovery: first, a destination-visible HOLD

Status: 04g-3b-7c-a, inert fail-closed precursor (2026-10-17). Not permission to move, overwrite, delete or sync local files.

## Why this is necessary

PR #121 journals history relocation in the **source** vault, but a separately opened Core with only the **destination** vault registered cannot see that journal. PR #124 safely refuses managed cross-vault primary-file moves before mutation. Removing that refusal merely because two JSON records exist would reintroduce a P0 race: writing separate files in two vaults is not an atomic transaction, and another Core can perform an operation between the writes.

## First concrete slice — read-only local HOLD visibility

A reserved marker at `<vault>/.aquilum/history-moves/cross-vault-hold-v1.json` is recognized independently by every Core which opens that registered vault. It does not require either Core to know the other root and is not written by production code.

- Version 1 marker fields: `schema=1`, UUID `operation_id`, `side=source|destination`, normalized source/destination vault identities and note names, expected primary and history-inventory BLAKE3 digests.
- Read limit is 4 KiB with exact schema and unknown-field rejection. Wrong vault/role, malformed UUID, unsafe path, future version, invalid digest, oversize, symlink and Windows reparse marker all fail closed.
- A present valid marker means **HOLD**; an unreadable or corrupt marker also means **HOLD/error**. Neither permits a write or implies that a matching peer marker exists. The existing `HistoryService` history-read/write/maintenance admission sees the marker after restart. Both the old `pending-v1.json` and the new hold marker are checked without ignoring corruption in either.
- The existing PR #124 File Gate denies cross-vault primary relocation regardless of marker contents. This slice deliberately does not change the primary-file writer or activate a coordinator.
- Tests create the marker only inside temporary fixtures. No production component may create or remove it until a separate safety review designs its lifecycle.

## Required follow-up 04g-3b-7c-b — actual bilateral admission

Before enabling moves across roots:

1. Establish a **real cross-process admission barrier** covering *both* independently opened registered vaults and all cooperating mutation entrypoints, with a stable locked OS file/directory identity. A mutex local to one Core or two O_EXCL pathname checks does not suffice.
2. Under that barrier, validate the source primary and both version inventories by pinned descriptor/handle with exclusion of symlinks, junctions and hardlink identity ambiguity; reserve collision-free destination names without overwriting.
3. Durable source PREPARED + destination PREPARED must reference the same operation and exact preimages; parent metadata persistence must be proven per platform. The lock stays held across revalidation and primary mutation. An absent, changed or forged receipt is never a positive authorization.
4. Write a durable, ordered transition journal for primary published, history transferred, and both verified. A missing acknowledgement requires read-only reconciliation; never replay a rename or clear the losing history opportunistically.
5. Independent source-only and destination-only Core tests must cover crash immediately after each prepare, after the primary rename, after one history file and before final acknowledgement; external-process races, destination collisions and a lost lock must preserve every version or stop for operator review.
6. Windows directory metadata/power-loss durability remains unproven; on unsupported filesystems report HOLD rather than silently treating successful process exit as durable commit.
7. Provide an explicit operator resolution/forensics path for incomplete transfers. Do not automatically delete, roll back, prune history or unlink any marker.

## Current acceptance

- `history_cross_vault_hold_*` tests are run by the existing Linux/macOS/Windows pinned-read CI matrix, alongside the cross-vault File Gate denials.
- A destination-only Core reopened with its own app-data refuses `HistoryService::try_saved` while its local HOLD marker exists, without changing the primary document or recorded version bytes; corrupted markers also refuse.
- An intact pair of matching hold markers still **never** grants mutation authority.

References: [Cross-vault restriction in ADR-SYNC-005](adr-0005-inprocess-history-files-interlock.md); [SYNC-04g native File Gate boundary](adr-0006-cross-process-mutation-boundary.md); [canonical plan](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11).
