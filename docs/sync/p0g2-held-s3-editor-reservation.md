# P0g-2: retain a closed-editor reservation for one staged S3 operation

Status: internal preparatory code with test-only writer composition. Not an
S3 automatic-apply coordinator or native durability receipt. Public local
release flags and installed-host policy are unchanged.

## Gap after #348

The common cooperative writer now retains one OS custody lifetime over
preparation, File Gate, HistoryService and postflight. The old preimage
revalidation method, however, releases its DocumentHub reservation when that
individual check returns. A subsequent editor open can therefore occur
between that check and a later write. The generic preparation value in #348
needs to retain the actual closed-document reservation, not just a copy of
file IDs or a journal claim.

## Implementation

`prepare_reserved_s3_replace` creates a non-Clone, must-use
`ReservedS3Replace` as the preparation value of the existing #348 writer.
It does not acquire a second OS lock, expose a File Gate permit, write a
workspace file, fetch from S3 or change a cloud baseline.

It checks the caller's exact planned operation and proposed UTF-8 bytes
against the existing Core-approved S3 profile/VaultId, immutable object key,
verified private blob cache and protected original. Private database paths
must remain outside the registered workspace. An installed-host Core with
no implemented protected writer is refused before a claim is consumed.

The object retains DocumentHub's existing `SyncReservation` and a pinned
root. Preparation uses bounded descriptor-relative reads to check physical
file ID, old hash and exact original bytes. It then uses the existing V7
`record_existing_replace_attempt_once` transaction for baseline/path/document
binding and non-replayability; there is no second journal format or writer.
Post-claim checks fail closed without deleting or resetting the claim.

The surrounding trusted synchronous host callback retains this object
through the actual File Gate and HistoryService calls. Postflight compares
File Gate's returned hash, two bounded pinned reads, the exact cached target,
original record, previous confirmed baseline, profile and one-shot attempt.
It rejects another Core even when that Core names the same physical root.
Dropping the object releases only the local editor reservation. It never
releases the durable attempt or permits an automatic retry.

## Evidence required before acceptance

The dedicated Linux/macOS/Windows workflow executes six real temporary
Core/SQLite tests and the existing single-custody writer regression suite:

- During preparation and postflight, actual `DocumentHub::open` returns
  `SyncReserved`; the reservation can be reacquired only after return.
- An already open editor prevents a new claim and leaves original bytes.
- A different physical file with identical bytes is rejected before a claim.
- Different proposed bytes or a foreign registered profile are rejected.
- A HistoryService failure after primary write skips postflight, preserves
  old baseline/original and the irreversible claim, and releases only the
  transient editor reservation.
- An injected postflight change returns NeedsReview, never remote success.
  Baseline stays old, journal stays Staged, and generic mark_applied is denied.

The fixture uses synthetic remote HEAD/baseline metadata, not a new live S3
transfer. It uses the real Core, profile binding, external cooperative lock,
File Gate, HistoryService, protected-original store and V7 journal. Native
Rust is unavailable in the local working container; GitHub exact-head CI
is required, not assumed.

## Still missing

This preparation function must run inside #348's trusted OS-lock callback;
it does not independently prove the caller is holding that lock. Only its
tests currently connect it to a writer. It is not exposed in desktop/UI or
an S3 apply API. It validates private persisted state, not current remote
HEAD/ancestry or grant revocation during a later network roundtrip.

DocumentHub reservation excludes editor sessions in this Core, not arbitrary
standalone File Gate users, other processes, or hostile same-user filesystem
modifications. Cooperative OS custody still provides the separate
participating-process exclusion. Hash/identity checks are not atomic inode
CAS against uncooperative writers.

The actual native exchange/ReplaceFileW adapters, parent and target fsync,
crash/lost-ACK readback, independently durable native receipt and baseline
commit remain separate gates. A successful pinned postflight is an
observation, NOT proof of power-loss durability or authority to mark the S3
revision applied. Preserve LocalDownloadNeedsNativeReceipt and all current
installed-host HOLD behavior until those gates are independently accepted.
