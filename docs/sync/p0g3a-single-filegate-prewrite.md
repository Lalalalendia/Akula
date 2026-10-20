# SYNC-APPLY-P0g-3a — one Core File Gate publisher, prewrite physical identity

Status: **private cooperative boundary + disposable test suite only**.
No installed-host S3 write entrypoint, no OS-native receipt, no remote baseline
promotion, no release feature or network provider code.

## Observed gap after #348 / #352

The newly retained `ReservedS3Replace` verifies the physical original before
the V7 irreversible attempt claim and performs pinned readback after the
canonical File Gate / HistoryService write. However, after preparation
returns, the guarded writer's `write_checked_history` previously relied on
a pathname/raw-byte CAS in `AtomicWriteFile`. A non-cooperating writer could
rename the old inode away and insert a **different inode with identical old
bytes** in this interval. Byte-only CAS would not identify the substitution.

The separately accepted Unix swap and Windows ReplaceFileW test probes are
**not** the File Gate implementation. Chaining either probe followed by the
real File Gate would publish twice and could invalidate history provenance.
Do not introduce that design.

## Change

The existing guarded path now delegates to a narrow private
`save_existing_note_with_prewrite_under_custody` bridge:

1. Keep the same previously registered OS custody lock for the *entire*
   operation, with an optional non-Clone retained editor reservation.
2. Validate existing note path; prepare and durably claim the exact V7
   replacement once, preserving the bound original outside the vault.
3. Acquire the *real* Core File Gate mutation mutex.
4. **Inside that mutex**, run the trusted one-shot
   `ReservedS3Replace::inspect_before_write`: recheck live registered
   Core/root/profile/baseline/intent/claim/original and require exact
   original physical file identity, original byte hash and bytes.
5. The one existing `write_file_atomic_impl` performs the primary write.
   No native experimental writer is invoked before or after it.
6. Release File Gate mutex; persist actual HistoryService as before; perform
   pinned postflight while OS custody and closed editor reservation stay held.
7. Drop preparation state and OS custody lock before UI/search notifications.

Existing `save_existing_note_with_checks_under_custody` retains identical
behavior by passing a no-op prewrite hook. No local editor route changes.

If prewrite refuses **after** a durable V7 claim, return `NeedsReview`, not
`Denied`, and retain the once-only claim, old confirmed baseline and
protected original. No file commit, no postflight, no cleanup or retry.
`StageDownload` remains `Staged` and generic `mark_applied` stays
`LocalDownloadNeedsNativeReceipt`.

## Tests

Run `s3_reserved_replace::tests` plus `guarded_note_write::tests` with
`--features sync-s3` on Linux, Windows and macOS using the existing
`Sync reserved S3 preparation` workflow.

- Positive: a real File Gate update and real HistoryService persistence;
  one prewrite callback exactly while the Core File Gate mutex is held and
  the DocumentHub reservation blocks new editors; postflight once; old
  baseline not acknowledged.
- Negative: after durable V7 preparation, a deterministic same-user test
  swaps in an identical-byte foreign inode. The innermost prewrite check
  rejects the foreign identity **before** the one File Gate publisher.
  The original is retained under another test filename; no new remote
  baseline, no postflight, no automatic V7 replay.

These are controlled disposable fixtures. They do NOT authorize or test a
production S3 overwrite path.

## Security limits and P0g-3b gate

This improves identity checking for **cooperating Core writers** by moving the
last preflight under the actual serialization mutex. It still does not make
`AtomicWriteFile` an **atomic inode compare-and-swap**: a hostile writer
with the same OS credentials may replace the pathname after the check and
before the filesystem publication syscall. The underlying atomic-write
library remains the one primary writer. No new native OS swap is wired.

There is no trusted host-wide receipt linking the **actual** postpublication
OS file ID and file/directory sync boundary to HistoryService's committed
version. A successful in-memory `FileWriteResult` or pinned readback is not
power-loss durability evidence and must not advance `ConfirmedBaseline`.

P0g-3b must decide the single canonical File Gate native publication
primitive per OS, add file/parent durability and error-state handling,
persist an immutable independently reverified receipt, and only then
consider a separately reviewed exact-receipt/remote-DAG baseline transition.
Unsupported and ambiguous outcomes remain HOLD.
