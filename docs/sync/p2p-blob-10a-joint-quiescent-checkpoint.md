# P2P-BLOB-10a — joined quiescent FsStore staging checkpoint

**Status: experimental Linux-only draft**, stacked on [P2P-BLOB-09b PR #343](https://github.com/Lalalalendia/Akula/pull/343) → #337 → #327 → #321 → #314 → #301. No production Akula Sync Core, user files, S3, File Gate, Roha/noq forks or Windows/Android storage paths changed.

## Why this matters

P2P-09a scanned external data files and P2P-09b separately inspected the redb database, including a real small blob stored entirely inline. There was no joined check that all three views agreed on the same hashes, reserved bytes and verified states. A valid inline blob has no external .data file, so the standalone 09a audit conservatively HELD even if 09b verified its bytes.

P2P-10a performs a **serial, fail-closed combined observation** under the SAME exclusive DurableLedger owner and SAME bounded staging mount guard, assuming iroh-blobs FsStore is completely shut down. This is NOT an atomic filesystem snapshot, not race-free fd-relative custody, not a power-loss barrier and not File Gate authority.

## Implementation

1. The pinned read-only redb 4.4.0 audit now privately retains hashes proven as complete inline BLAKE3-exact payloads. The existing standalone public 09b audit continues unchanged. No writable DB or repair API added.
2. The existing standalone 09a external scan also remains strict and unchanged. A crate-private version may accept ONLY the hashes validated in the *same* redb pass, allowing absence of corresponding .data files while rejecting a stray duplicate external file or outboard, foreign hashes, residual temp, invalid sizes and modified external BLAKE3.
3. A new Linux-only joint_staging_audit module compares one ledger snapshot with redb complete record count, inline + external verified object counts, exact byte sums and generation. It re-hashes the original DB after finishing the external scan, checks journal snapshot equality again and checks staging mount identity before and after.
4. JointEvidenceOnly permanently exposes file_gate_ready=false, bao_verification_pending=true, atomic_filesystem_snapshot=false. **Positive evidence never grants Akula File Gate publication.**
5. Improve P2P-07 OS lock diagnostics without weakening exclusion: WouldBlock remains CompetingOwner; unexpected OS failures are returned as Io(error). Add 96 repeated clean owner handoffs while preserving the same durable reserved bytes. Previous intermittent CompetingOwner is not declared fixed just because this test passes.

## Required exact-head CI

- Existing pinned Roha/noq, direct QUIC security and deny-PUSH, SIGKILL, bounded ext4 ENOSPC, missing-mount, external corruption and redb inline/corruption tests must remain successful.
- Against the SAME disposable 64MiB real ext4 image, independently join the persisted 1,048,593-byte actual external FsStore and its journal, and the persisted real inline small FsStore and its own journal. The ledger bytes and source DBs must remain identical before and after audit.
- Foreign valid DB paired with a different valid durable journal must HOLD. Inject only a disposable duplicate external .data for an attested inline DB blob and verify HOLD; a separately corrupted throwaway redb copy must HOLD. Clean up ONLY injected test files.
- Unmount/remount the entire same ext4 image, then rerun both complete joined audits against the same disk journals. Any unknown data or lost mount must be rejected; no automatic deletion/repair.
- The four required GitHub workflows must be green on exact latest head. Retain any intermittent lock diagnostic anomaly, even when retrying results in success.

## Non-goals and remaining barriers

**This is NOT an atomic three-component storage snapshot.** The ledger OS lock governs cooperating processes only; redb shared read transaction is not atomic with the external pathname scan; before/after BLAKE3 and mount identity checks detect accidental but not all malicious path changes. We still lack kernel-enforced ownership/fd-relative custody and safe untrusted concurrent file writes.

Bao outboard proof tree integrity, verified partial bitfields, DataLocation::External paths, unknown files outside the inspected store root, real disk crash/power loss, signed VaultId/RevisionId/DeviceId ↔ EndpointId authorizations, Windows/Android physical quotas and File Gate integration remain pending.

**The successful result is research-only.** Keep all stacked PRs draft/unmerged. Next require true fd-relative and exclusive backing-store custody, authentic Bao proof and interrupted ext4 writer kill testing before product rollout.
