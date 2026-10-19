# P0g-1: one cooperative custody lifetime for preparation and postflight

Status: internal writer refactor plus disposable integration tests. This is
not an installed S3 executor or a native publication receipt.

## Gap addressed

The accepted P0e fence and P0f File Gate tests invoked preparation and the
writer as separate operations, each acquiring and then releasing the same
external OS lock. The existing guarded writer already retained that lock
across primary file publication and HistoryService, but could not retain it
across caller preparation and postflight. Independent successes did not prove
one continuous interval.

The existing `save_existing_note_under_custody` now delegates to a common
`save_existing_note_with_checks_under_custody` implementation using no-op
checks. The latter takes synchronous trusted preparation and postflight
closures. It retains one verified external custody lifetime in this order:

    acquire existing registered custody
      -> validate existing note path
      -> preparation (e.g. immutable V7 attempt + retained reservation)
      -> existing Core File Gate primary CAS
      -> existing HistoryService persistence
      -> postflight
      -> drop preparation state
      -> custody postchecks / unlock
      -> search and UI notifications

The preparation value is borrowed by postflight and dropped before unlocking;
it is not returned as a reusable writer capability. File Gate and HistoryService
retain their existing internal lock order; no second OS-lock acquisition is
introduced. The old writer and its private-namespace/alias tests are retained.

## Failure contract

- Missing or mismatched custody, or unsafe note path: `Denied`, with neither
  preparation nor the primary writer entered.
- Once preparation starts, any error is `NeedsReview`: preparation might have
  committed a durable one-shot claim before reporting a failure. Do not equate
  unchanged primary bytes with permission to try again.
- Failed primary or history: skip postflight. History failure can occur after
  new primary bytes are visible; the protected original and old baseline remain
  necessary for inspection.
- Failed postflight or custody postcheck: `NeedsReview`, never a successful
  remote ACK. A successfully recorded HistoryService change is still announced
  after unlock, including when later postflight fails. Search/UI callbacks never
  execute under the held external OS lock.

## New test coverage (CI acceptance required)

Five substantive tests plus one explicitly invoked child helper exercise the
real Core, HistoryService, external OS lock, protected-original SQLite and V7
one-shot journal. They do not use a real S3 transfer: prior provider metadata
is marked synthetic in the fixture.

The positive test durably records the replacement attempt during preparation,
performs the existing real File Gate CAS, and checks file bytes and history
in postflight. An independent OS process is unable to acquire the lock during
both preparation and postflight, and can acquire it after return. Retained
preparation state remains alive until postflight; UI callbacks re-acquire the
lock only after it has been released.

Negative tests inject failure after committing preparation, in HistoryService,
and in postflight; a separate test rejects unbound custody before any work.
After each claimed attempt the journal is reopened, the old baseline and
protected original are checked, phase remains `Staged`, a second attempt is
refused and generic `mark_applied` remains `LocalDownloadNeedsNativeReceipt`.
The child observer has a 30-second deadline and a kill/wait guard.

## Remaining product boundary

These host callbacks are not plugin/UI/remote-supplied code. The common writer
still uses its current File Gate implementation, not the native Unix exchange
lab from #339 or the Windows ReplaceFileW experiment #345. It does not guarantee
power-loss durability, enforce inode CAS at the OS syscall, mint a native
receipt, validate the remote DAG, or commit a Sync baseline. Uncooperative
same-user filesystem changes are not excluded by an advisory lock.

A real S3 coordinator must supply the independently verified revision and
closed-editor reservation, bind exact root/file identities at publication,
perform platform-specific file/parent sync and independent postflight, and
persist a receipt bound to the actual HistoryService result. This patch does
not implement that full policy or connect the helper to automatic cloud apply.
The public installed release, mandatory host policy and receipt/baseline guards
are not loosened. Existing #340 and native #339 evidence remain complementary,
not a reason to declare the whole sync chain ready.
