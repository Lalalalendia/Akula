# G1-01 — one non-authorizing cold oracle and machine-readable verdict

**Status:** draft/CI pending. This is **not** a native receipt, an S3 writer or a storage durability proof.

Parent requirement: [SYNC-CRASH-MASTER-01](https://app.notion.com/p/3ff32a84beec8127a9aaf14101ef45f1). It follows the accepted selective-domain loss matrix [#457](https://github.com/Lalalalendia/Akula/pull/457) and can be tested while QEMU hard-off [#466](https://github.com/Lalalalendia/Akula/pull/466) is still Draft.

## Boundaries and format

The ONLY substantive decision engine is the existing `inspect_cold_s3_replace_with_history_hold` in `s3_reserved_replace.rs`. Its result is mapped by a **test-only** `cold_oracle_report.rs`, compiled under `#[cfg(test)]`. Nothing is installed as a production API, MCP command, receipt writer or S3 remote acknowledgement path.

Report marker (emitted only when the OS child is run in an explicit `oracle-hold` or `oracle-deny` laboratory mode):

```json
{
  "schema": "akula.sync.cold-oracle.v1",
  "operation_id": "<UUID>",
  "verdict": "LOCALLY_CONSISTENT_HOLD",
  "reason_code": "CORRELATED_NOT_DURABLE",
  "authority": "none",
  "receipt_issued": false,
  "automatic_apply_allowed": false,
  "remote_ack_allowed": false,
  "baseline_promotion_allowed": false
}
```

Negative inspector result => `DENY` / `EVIDENCE_REJECTED`; a returned review for a different operation => `DENY` / `WRONG_OPERATION`. An unexpected future inspector status fails closed. These generic codes **do not pretend to distinguish V7/primary/history/WAL root causes yet**. Do NOT parse free-form `FileCommandError` messages to infer domain or permission.

Reports omit file paths, original contents, passwords, provider metadata and arbitrary error texts. They have no signature, power-cut assurance or authorization. Consumers MUST verify the exact named native test exited successfully, that exactly one test passed, that the operation matches, and that a single valid JSON envelope was emitted.

## Real tests on disposable Core

- Positive control: real guarded File Gate + exact immutable HistoryService + V7/V8 and independently readable original; writer/Core handles dropped; a new OS process emits `LOCALLY_CONSISTENT_HOLD`.
- Negative control: the same external history is overwritten with different bytes of the same length; an independent OS process emits `DENY`.
- Existing journal remains `Staged` with the previous baseline; V7 one-shot claim and independent bounded original remain; `mark_applied` still returns `LocalDownloadNeedsNativeReceipt`.
- Unit controls: mismatched operation and rejected evidence always return deny with every authority bit false.
- No production file writer, no remote transport, no baseline migration and no automatic repair.

## What G1 still requires after this PR

Expand the actual inspector to expose *typed read-only failure domains* (V7 absent/corrupt, V8 mismatch, history missing/tampered, primary identity/bytes mismatch, original missing, baseline drift, registered root mismatch, WAL/DB unavailable) without a second source of truth. Keep the broad `EVIDENCE_REJECTED` compatibility code until native error contracts are reviewed. Reuse the exact envelope for Linux block replay, Windows VM, SQLite faults and QEMU. Pair each report with host-owned scenario/trace provenance, exact git HEAD, FS, volume mapping, cache policy and actual observed flushes.

**Gate:** exact-head Required Rust, Main PR gate and Reserved S3 Linux/Windows PASS. No PR merge or G1 acceptance based solely on an added schema or a CI workflow that did not run the named test.
