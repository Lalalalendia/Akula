# SYNC-APPLY-P0f — real guarded File Gate after one-shot S3 claim (test only)

**Status: disposable test-workspace evidence, not a production S3 apply path.**

After PR #300, #316, #328 and #329, Akula can verify a remote existing
revision, stage it privately, capture the exact local original, and persist a
non-replayable update attempt. None of these preparatory actions used the
real primary existing-note File Gate writer.

This step adds **only two `#[cfg(test)]` Core integration tests** to the
already existing S3 protected-preimage fixture. It deliberately does not add
an API that can apply remote content, change the standard installer or
advance a Sync baseline.

## What these tests actually do

- Create a disposable, privately registered vault, actual Core, profile,
  external OS lock, protected-original store, S3 journal V7 and verified
  cached blob. No real user vault is selected.
- Capture exact original bytes under the existing pinned read gate; persist
  a once-only update attempt under the same registered OS custody.
- Invoke **the current** `guarded_note_write::save_existing_note_under_custody`,
  which is already wired to Core's File Gate + HistoryService. It is not a
  new implementation of file writing, lock ordering or history snapshots.
- On successful File Gate/HistoryService completion, inspect the new file,
  actual history entry, unmodified old Sync baseline, `Staged` journal
  phase, immutable protected original and post-restart non-replayable claim.
  `mark_applied` must still refuse with
  `LocalDownloadNeedsNativeReceipt`.
- Inject a **real HistoryService persistence failure after the primary
  File Gate write**. The guarded API must report `NeedsReview`, not
  successful cloud synchronization. Check that new bytes can be visible
  despite failed history, yet previous cloud baseline and protected original
  remain intact, and CAS denies an automatic replay.

## Explicit security and durability limits

These tests run only inside a Rust test binary; no S3 automatic-apply API is
exposed and no remote content is published by the released application.
File Gate's existing `AtomicWriteFile` path is exercised, but the tests
cannot claim power-loss durability or independent parent-directory fsync
across Windows/macOS/Linux. They also do not protect user-writable Markdown
from hostile same-UID processes or prove installed-host broker authority.

A persistent V7 attempt is **not** a post-publication native receipt. Even
after an apparently successful File Gate save and HistoryService callback,
the remote StageDownload is unconfirmed. On lost ACK, cross-process restart,
history failure or OS-lock postcheck failure, the correct next action is
`HOLD / NeedsReview`, not another attempt.

## Next production gate

Add platform-specific parent/target fsync and pinned post-publication
readback, plus an immutable native observation tied to the V7 attempt and
actual HistoryService result. Test process termination at before-write,
after-primary/before-history, after-history/before-receipt, and
after-receipt/before-ACK, including two independent Core appdata instances.
Only a separately reviewed, exact-version transactional commit may later
advance a baseline. Do not just remove the StageDownload
`LocalDownloadNeedsNativeReceipt` guard.
