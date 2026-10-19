# SYNC-P0g-3d — native durable receipt and cold-start recovery

**Status:** design-only; production S3 apply remains HOLD.  
**Dependencies:** #358 (File Gate identity), #363 (strict HistoryService witness), #385 (Core/path-bound custody permit), SQLite V7 one-shot journal.  
**Platforms:** Windows and Linux. macOS Frozen/P2 per MAC-HOLD-01.

## Goal

Associate exactly one authorized local replacement with a verifiable persisted receipt that ties together the **protected original**, **single File Gate primary publisher**, **post-publication file identity**, **exact HistoryService version**, and **SQLite V7 attempt**. Point-in-time observation is not proof of power-loss durability.

## Threats and invariants

- No replay of the same V7 one-shot claim after ambiguous failure, even after cold restart.
- Never destroy or garbage-collect the only independently restorable original while any phase is uncertain.
- No second primary publication path, history bypass, or crossing Core/note identities.
- All authorization is specific to registered Core, vault, checked pathname, attempt, original identity and expected hash.
- Hostile same-UID filesystem modification and power-loss behavior are **not** solved merely by an OS lock or rename.
- No `StageDownload::mark_applied`, remote ACK, automatic S3 overwrite, or `ConfirmedBaseline` promotion in this stage.

## Proposed durable state machine (subject to implementation review)

| State | Meaning | Restart decision |
|---|---|---|
| `PREPARED` | V7 claim and independently retained protected original verified; no primary authorization issued | Reinspect original and claim; HOLD |
| `PRIMARY_UNCERTAIN` | Primary write entry or publication might have happened | Compare retained original and destination physical identity/bytes; NeedsReview |
| `HISTORY_UNCERTAIN` | Primary publication observed; history write/witness may be incomplete | Cold re-read history and primary; NeedsReview |
| `RECEIPT_PENDING` | All evidence observed; durable receipt not yet independently verified | Revalidate all inputs; HOLD |
| `VERIFIED_LOCAL` | Restart verifier accepts correlated persisted evidence | Eligible only for a *later*, separately reviewed apply/ACK gate |
| `NEEDS_REVIEW` | Any mismatch or ambiguous durability | No automatic retry or baseline promotion |

A transaction must persist the phase *before* starting an irreversible step, and provide failure injection at each boundary. The exact order of protected-file, journal, primary-file, history and directory synchronization must be proven separately for Windows and Linux rather than assumed from SQLite atomicity.

## Ordered delivery

1. **P0g-3d-01 — byte-level audit and spec.** Locate real File Gate atomic writer, protected-original strategy, SQLite V7 schema, and HistoryService persistence. Document file/directory fsync semantics and every power-loss window.
2. **P0g-3d-02 — retained original.** Independent, re-readable snapshot whose byte hash and physical identity can be checked after crash; no premature cleanup.
3. **P0g-3d-03 — native receipt.** Define versioned receipt with attempt ID, Core/vault/path, original identity/hash, expected and actual destination hash/identity, exact history version, V7 claim and durable phase.
4. **P0g-3d-04 — cold-start verifier.** New process reconstructs authority from disk, treats unverifiable or mismatched evidence as NeedsReview, never assumes memory-held lock survived.
5. **P0g-3d-05 — Windows/Linux fault injection.** Crash/terminate before and after each durable phase, physical pathname swap attempts, different Core, changed original, stale history witness, and two-process contention.
6. **P0g-3d-06 — independent gate review.** Prove recovery and absence of unauthorized baseline promotion; any future S3 enablement needs a distinct approved change.

## Acceptance

Fresh-head Windows/Linux CI must demonstrate negative and positive cases, deliberate termination/restart, preserved originals, exact SQLite/history/primary correlation, and no unsafe retry. A green unit suite alone cannot establish power-loss safety. macOS is excluded without deleting source support.

**HOLD:** design doc is not proof of implementation and does not authorize S3 apply.
