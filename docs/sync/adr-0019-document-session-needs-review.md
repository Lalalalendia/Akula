# ADR-SYNC-019 — Sticky document-session HOLD after ambiguous native write

Status: **opt-in preparation only**, not a durable recovery receipt or user-facing S3 activation (2026-10-18).

## Problem

Merged PR #210 introduced an explicit cooperative existing-note writer that
returns `NeedsReview` if the native note file or HistoryService may already
have changed without a trustworthy completion acknowledgment. Draft PR #215
proposes mapping this outcome to `DiskError::Other { code: "needs_review" }`
for the real `DocumentHub` editor route.

Previously, `DocumentSession::write` returned that error while leaving
`dirty = true`. Subsequent client edits, watcher-triggered
`DocumentHub::reconcile_entry`, and scheduled save operations could try
another physical write **before anybody investigated the first one**.

## Narrow internal fix

- On a `DiskError::Other { code: "needs_review" }`, set an in-memory,
  **sticky** `DocumentSession.needs_review` and keep user edits `dirty`.
  Never advance the last confirmed sync-point metadata.
- While held, `DocumentSession::write` and `reconcile` fail before
  reading or modifying disk. `DocumentHub` refuses to reschedule this
  session in watcher reconciliation and refuses subsequent disk saves.
- Editor CRDT changes may still be appended to its app-private local
  durable delta log and remain in the dirty replica. Neither new edits nor
  a matching file BLAKE3 can implicitly clear HOLD.
- A fault-injection test first publishes primary bytes but loses its ACK,
  then simulates a concurrent external disk write, a reconciliation request,
  a save retry, and another user edit. The original write count remains
  exactly one; the old confirmed sync point remains unchanged.

## Outstanding production gates

**This HOLD is process-local.** Closing/reopening a session or restarting
Akula does not yet restore it. Draft PR #215 must not be activated as a
normal desktop/editor route until app-data has a trusted, restart-durable
HOLD with an explicit operator-reviewed clearance flow.

The legacy native `native/src/note/save.rs` save channel is separate and
currently automatically retries `Outcome::Failed` after 5 seconds in
`native/src/note/mod.rs::fail`. Never map an ambiguous guarded failure
into that transient outcome. A future native editor adapter needs a
distinct no-retry review state; no such adapter is enabled here.

This ADR is not an S3 overwrite/delete/GC permission, an interprocess
malicious-writer guarantee, Windows power-loss receipt, conflict resolution,
or confirmed `StageDownload` baseline.
