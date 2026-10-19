# SYNC-APPLY-P0e-WIN2: real Windows process termination at ReplaceFileW boundaries

**Test-only, stacked on P0e Windows #345.** This does not activate S3
downloads as filesystem writes. It extends the existing Windows-only
`native_windows_replace_probe::tests` module, which is only compiled
under `cfg(all(test, windows))`, and only touches disposable
`tempfile` directories. No shipping baseline permission changes.

## Three independent process boundaries

A parent creates an isolated document, synthetic previously committed
revision, `JournalPhase::Staged` download and opens no live journal handles
during the child's mutation. The child opens the real V7
`SqliteSyncJournal`, persists
`record_existing_replace_attempt_once` with root/target identities and the
exact bound original, then enters the actual Win32 `ReplaceFileW` probe.

The child signals a bounded barrier using a create-new synced marker,
then blocks. The parent calls `Child::kill` (Windows TerminateProcess),
waits and independently reopens both filesystem and journal:

- `before_replace`: stage exists with new bytes; original remains under
  `file.md`; backup must not exist.
- `after_replace`: stage disappeared; `file.md` is new; named backup
  retains the original bytes and exact original physical file ID.
- `after_postflight`: native identity and byte readback returned;
  exactly the same disk names exist, but no native sync receipt exists.

Each case requires unchanged `ConfirmedBaseline`, journal still
`Staged`, immutable original identity in the one-shot V7 claim,
`ExistingReplaceAttemptAlreadyRecorded` on replay, refusal of
`mark_applied` with `LocalDownloadNeedsNativeReceipt` and recovery
queue inspection rather than silent retry. The child must reach its
specific barrier before termination; early exit fails with a retained log.
A 30-second deadline and kill/wait guard prevent indefinite hanging.

Exact-head Windows CI acceptance should show the marker
`AKULA_WIN_REPLACE_TERMINATE_PROCESS` for all three cases.
The existing `native_windows_replace_probe::tests` step runs the
nested tests without a workflow or shipping-source change.

## Explicitly not proved

The previous `catch_unwind` test covers only process-local panic.
This follow-up genuinely kills a separate Windows test process, but
does **not** simulate machine power failure, NTFS metadata durability
across remount, hostile same-UID writer races, backup security outside
the user vault, file sharing and ADS/EFS metadata invariants, or atomic
identity CAS. The fixture baseline is synthetic, not an authenticated
remote S3 DAG. No real Core/HistoryService writer is called by this
isolated `ReplaceFileW` probe.

No native journal receipt is minted by this code. These tests must not
be cited as evidence that `ConfirmedBaseline` may advance.

## Integration gates after this falsifier

P0g #348's existing File Gate/HistoryService transaction and these
OS-native probes remain independent. Future production work must bind
registered host custody, closed-editor state, V7 claim, protected original
outside the vault, *actual* File Gate mutation, history, OS-specific
postflight and typed durable receipt in one lifetime, then prove safe
startup recovery before enabling S3 apply. Unsupported or ambiguous
results remain permanent HOLD pending explicit human review.
