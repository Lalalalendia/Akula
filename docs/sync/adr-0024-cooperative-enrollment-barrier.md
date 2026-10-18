# ADR-SYNC-024 — Atomic cooperative enrollment against File Gate and direct HistoryService

Status: opt-in, fail-closed race hardening after PR #239. No remote overwrite/delete authorization.

## Observed race

Previously `activate_registered_s3_editor` verified existing S3/custody identities and set `DocumentHub` participant HOLD without holding either the process-wide File Gate mutex or `PROCESS_HISTORY_FILES`. An ordinary writer could have checked HOLD=false and entered the File Gate, or a direct HistoryService mutator could have checked false before waiting on the history mutex. A concurrent activation could then publish its new participant policy while that unguarded mutation was in flight.

## Narrow change

1. Verify the previously registered S3 profile, distinct VaultId, physical root and pinned external custody evidence without changing any file.
2. **Before acquiring File Gate**, check the DocumentHub sessions mutex for open/unsent buffers and arm the fail-closed prewrite root. DocumentHub save holds sessions while entering File Gate; reversing this order would deadlock. If a later enrollment step fails, DocumentSession attempts in that temporarily armed root require durable intent and one-shot capability but are refused by the unactivated S3 route rather than silently falling back.
3. Acquire `PROCESS_FILE_MUTATIONS` **before** the guarded route registry and HistoryService mutex. This waits for a currently active ordinary File Gate primary-file critical section to exit. Preserve `File Gate → HistoryFiles` lock order; never acquire an external OS lock inside the enrollment critical section.
4. Hold `PROCESS_HISTORY_FILES` via `HistoryService::with_participant_enrollment_fence`, rejecting poisoned locks. While both local barriers are held, insert the SQLite v3 participant record with FULL sync, and only then publish the guarded editor route.
5. `HistoryService::try_saved`, `name`, direct move/trash/restore, and retention perform a **second** participant-HOLD check after entering the shared HistoryFiles mutex, so a writer blocked behind enrollment cannot act on a stale pre-lock observation. The narrow `try_saved_with_custody` exception still requires the sealed permit created inside a verified held OS lock, preserving checked history for the guarded writer.
6. Regression exercises immediate admission refusal for a live DocumentHub session even when another thread holds File Gate (deadlock falsifier); actual S3-profile/custody activation while another thread owns the real File Gate mutex; and a HistoryService enrollment fence blocked by an in-flight history operation. It verifies the flag is not exposed until the prior critical section ends and that post-enrollment ordinary history mutation refuses.

## What this does NOT prove

The ordinary File Gate deliberately releases its process mutex BEFORE attempting HistoryService. An unguarded *legacy write that has already committed its primary bytes* may therefore be between those stages while activation occurs. This change prevents overlapping primary/history critical sections at the activation instant, but does **not yet** give an end-to-end publication lease or guarantee a truthful ordinary save ACK for that in-between case. The next release gate is a full transaction-lifetime admission epoch covering primary file + history, or a host-wide quiescence barrier that proves all in-flight publication attempts are drained. Do not present this as a complete atomic local edit transaction.

The barriers apply only to cooperating Core instances in one process and the bound Core's HistoryService. A second process or separate appdata root may bypass local HOLD. Hostile same-UID replacement of custody, direct native/OS writers and other File Gate bypasses remain outside this proof. No automatic S3 overwrite/delete, baseline promotion, GC, public S3 activation or HOLD clearance.
