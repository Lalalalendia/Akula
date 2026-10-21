# G1-02a — phase-typed rejection codes from the existing cold inspector

**Status:** draft, requires exact-HEAD Windows/Linux and QEMU CI. **Not** a durable receipt or authorization to perform a local/remote write.

References: [SYNC-CRASH-MASTER-01](https://app.notion.com/p/3ff32a84beec8127a9aaf14101ef45f1), [accepted G1-01 #474](https://github.com/Lalalalendia/Akula/pull/474), [typed report v1](p0g3d09-cold-oracle-v1.md).

## Architecture

The existing `inspect_cold_s3_replace_hold` still makes the full read-only primary/profile/V7/V8/original correlation decision. The history-enabled entry point now delegates to `inspect_cold_s3_replace_with_history_classified`. The original function signature still returns `Result<InertColdUnifiedReview, FileCommandError>` and maps typed failures back to the **same original FileCommandError**, preserving consumers such as QEMU, other cold OS tests and Windows/Linux callers.

The classified path is the **same** cross-store/history verifier, not an independent or permissive second oracle. A typed `ColdReviewFailureReason` is attached **at the exact failing check** and never recovered by parsing a human-readable string. A successful read still means only `LOCALLY_CONSISTENT_HOLD`; neither it nor any failed stage can authorize write/ACK/baseline.

The test-only envelope `akula.sync.cold-oracle.v1` retains schema and DENY/HOLD semantics, but `reason_code` can now explicitly report:

| Reason | Meaning |
| --- | --- |
| `PRIMARY_NOT_PUBLISHED` | V7 exists but current primary is not the target with a distinct identity; may still be the protected original |
| `V8_MISSING` | Primary target is visible but persisted V8 candidate is absent |
| `IMMUTABLE_HISTORY_REJECTED` | Native HistoryService could not validate the exact external immutable history version/bytes/identity |
| `CROSS_STORE_EVIDENCE_REJECTED` | Inner cold inspector rejected root, SQLite, V7, original, primary identity or V8 local evidence; finer subdomains **not yet** classified |
| `V8_UNAVAILABLE` / `V8_OPERATION_MISMATCH` / `V7_UNAVAILABLE` | Candidate/claim read or identity failed during history phase |
| `HISTORY_TARGET_MISMATCH` | V8 history digest diverges from claimed target hash |
| `REGISTERED_ROOT_CHANGED` | Registered root failed a before/after external history recheck |
| `POST_HISTORY_EVIDENCE_REJECTED` / `POST_HISTORY_EVIDENCE_CHANGED` | Full second-pass cross-store comparison failed or read-back changed |

**Important:** `IMMUTABLE_HISTORY_REJECTED` encompasses missing/tampered/unreadable versions; it is not a proof of a specific root cause. `CROSS_STORE_EVIDENCE_REJECTED` remains broad. Do not infer from these codes that a particular write was durable, that NTFS directory rename was persisted, or that hardware actually lost power.

## Positive/negative evidence

- OS child cold verifier after full V8 sees `LOCALLY_CONSISTENT_HOLD / CORRELATED_NOT_DURABLE`.
- Same-size corruption of HistoryService version produces `DENY / IMMUTABLE_HISTORY_REJECTED`.
- Claim-only V7, with protected original still under the primary name, produces `DENY / PRIMARY_NOT_PUBLISHED`.
- Primary + HistoryService present without V8 produces `DENY / V8_MISSING`.
- Unit tests ensure that every emitted result sets `receipt_issued`, `automatic_apply_allowed`, `remote_ack_allowed`, `baseline_promotion_allowed` to `false`, never includes raw paths or internal exception text.
- Existing cold child assertions still require `Staged` phase, previous baseline and retained original. The result is diagnostic only, and retry/ACK stays blocked.
- CI acceptance must use actual named Rust tests on exact-HEAD Windows/Linux `sync-s3`; Green workflow with 0 tests selected is not evidence.

## Remaining G1-02b, not claimed here

Differentiate inner cold failures at source: V7 absent versus V7 record corrupt, protected original missing, baseline drift, physical primary root/file ID mismatch, V8 local digest mismatch, SQLite/WAL unavailable, profile/root mismatch. Avoid a second store scanner for reasons. Add host-owned evidence provenance (fault model, exact SHA, FS, cache mode, flush boundaries) separately.

No S3 apply, remote ACK, `ConfirmedBaseline`, production File Gate or receipt-generation modifications in this PR.
