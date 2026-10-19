# P2P-BLOB-09b — pinned redb read-only metadata inventory

**Status: stacked experimental draft**, atop [P2P-09a #337](https://github.com/Lalalalendia/Akula/pull/337) → #327 → #321 → #314 → #301. No Akula main, S3, user files, File Gate, native app or Roha/noq fork changes.

## Why a database audit is required

In pinned iroh-blobs 0.103.1, a small blob can live entirely inside the redb database at FsStore/blobs.db, invisible to a pathname-only inventory. Equally, the complete BLAKE3 record may be stored externally with an inlined Bao outboard. [Upstream source for the four exact table definitions](https://github.com/n0-computer/iroh-blobs/blob/v0.103.1/src/store/fs/meta/tables.rs):

| Table | Key | Value | Current conservative rule |
|---|---|---|---|
| **blobs-0** | iroh_blobs::Hash | EntryState | Every hash exactly matches a durable verified reservation, data size agrees; partial and foreign external paths HOLD |
| **tags-0** | Tag | HashAndFormat | Bounded tags; raw tag targets must refer to admitted hash; unknown encoding or hash-sequence target HOLD |
| **inline-data-0** | Hash | byte slice | One exact complete inline record; validate expected length and full BLAKE3 over actual inline bytes |
| **inline-outboard-0** | Hash | byte slice | Must match exactly an EntryState inline-outboard indicator; byte count bounded; Bao proof remains unchecked |

The exact underlying redb type-name strings are pinned to the upstream Hash, Tag, HashAndFormat and EntryState Value implementations. The upstream EntryState from_bytes uses an unconditional panic on malformed postcard. Our audit uses small read-only opaque redb Value adapters and a deliberately narrow, size-bounded and panic-free wire decoder. Any unrecognized variant, bad varint, partial record or user-supplied External(path) is HOLD.

## Implementation

- Add **redb 4.4.0**, already present in Cargo.lock, as direct dependency only in the standalone Staya laboratory crate.
- Inspect existing file via **redb::ReadOnlyDatabase::open**, never Database::open, never FsStore::load, never a write transaction or repair operation. This forbids silent modification/recovery of the user's DB and takes a shared DB lock.
- Require the already-mounted, private, distinct-device volume root and exclusive P2P-07 journal owner; reject symlink/hardlinked/wrong-device/oversized DB before open. The DB is capped at 64 MiB, records and tags at 256, total inline payload at 16 MiB.
- Require EXACTLY the four pinned redb table names. Scan every record, compare with the pinned durable journal, catch orphans, missing entries, partials, wrong owned sizes, malformed EntryState variants, unexpected inline entries or missing required inline entries.
- Hash the full actual inline data and compare with BLAKE3 address and expected length. The database file itself is hashed before and after read-only inspection. The mount identity is checked at both ends.
- Return **RedbEvidenceOnly** whose file_gate_ready remains false and bao_verification_pending remains true. This evidence must NEVER authorize storage writes or protected-file publication.

## Real isolated ext4 tests

The same 64 MiB disposable Linux image and existing P2P-09a journals are used. Direct ext4 CI must:

1. Inspect the actual closed FsStore containing a 1 MiB+17B externally owned object, and match its redb metadata to the existing journal without changing source DB bytes.
2. Create another disposable genuine FsStore containing a short, inlined blob; create and fsync its own independent journal; inspect its exact inline-data entry and full BLAKE3.
3. Mutate ONLY a copy of the second DB to replace an inline payload without changing length and to add an unknown table. Both must HOLD; the original source and journal must be byte-for-byte unchanged.
4. Create another throwaway genuine FsStore with two blobs while the auditor journal authorizes only one, and prove an extra valid blob row in redb is HOLD.
5. Physically unmount and remount the ext4 image, then compare both original DBs with the same two durable ledgers again. All previous ext4 ENOSPC, missing-mount, external orphan/corruption, QUIC authorization, pinned Roha/noq and SIGKILL tests must remain green.

## Boundaries and blockers

**This remains research, not full end-to-end startup reconciliation.** The DB audit and external filesystem audit are separate and have no single race-free custody transaction. They do not validate Bao crypto or partial bitfields. A valid DB row and correct content hash do not prove that a foreign malicious or incomplete Bao proof is safe. The path checks are not yet fd-relative against privileged remount/rename races; Windows and Android need separate storage/lock mechanisms and trusted VaultId/DeviceId identity binding.

The inherited P2P-07 intermittent unexpected CompetingOwner on the initial CI attempt at PR #337, followed by a successful retry with the SAME SHA, remains a reliability concern, not a fixed bug. Future gating must distinguish lock errno and repeat OS-process reopen tests. Never weaken an actual exclusive OS lock.

Even after all exact-head CI passes, **keep this PR stacked, draft and unmerged**. Next: combined DB/external/ledger evidence under the same verifiable mounted snapshot, Bao/outboard verification, real unclean ext4 writer kill tests, and only then a prospective File Gate adapter.
