# ADR-SYNC-020 — Native note autosave must not replay an ambiguous File Gate write

Status: opt-in guarded S3 editor safety experiment, stacked after PR #215 and compatible with merged PR #219; not a production sync activation.

## Problem

The standalone native note adapter at `native/src/note/save.rs` uses `files::gate::write` rather than `DocumentHub`. Before this change, it mapped any non-conflict `FileCommandError` to `Outcome::Failed`; `Note::fail` then schedules another write after five seconds. With #215's `FileCommandError::NeedsReview`, a primary note or HistoryService version may already have changed. Repeating it without provenance would violate no-replay semantics even though #219 separately guards `DocumentSession`.

## Internal change

- Classify `FileCommandError::NeedsReview` as distinct `Outcome::NeedsReview`, not `Failed`.
- Use a per-open-note `SaveSafety` shared between all already-queued native saves. Its worker mutex remains held across one entire `File Gate → HistoryService` call, and once an uncertain result is seen all other queued workers refuse **before calling File Gate**, including jobs submitted before the first ACK arrives.
- UI scheduling uses a nonblocking atomic `needs_review` flag; no retry timer, tab-leave/flush replay or watcher reload after HOLD. Later edits can remain visible in the live editor but cannot implicitly clear HOLD.
- The result collector cannot replace an established HOLD with an out-of-order success ACK from a previously queued operation.
- Regression tests inject an ambiguous first worker result, prove a second worker closure is never called, and prove native UI retry and reload do not restart writes.

## Not yet safe for production

HOLD is process-local and does not survive restart or reopening a note. Pending changes from the standalone native note editor do not have the same app-private CRDT durability guarantee as `DocumentSession`; no automatic close/reopen safety is asserted. No recovery UI/clear-HOLD, stable participant registration, protected create/rename/trash, or broker-owned lock namespace. Non-cooperating same-user processes can still mutate files. Do not activate user-facing sync, automatic overwrite/delete, baseline advancement or GC on this evidence.

Related: ADR-SYNC-016, 017, 018, 019; PR #210, #215, #219.
