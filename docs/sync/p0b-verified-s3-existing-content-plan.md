# SYNC-APPLY-P0b — verified existing-file fast-forward candidate

**Read-only scope.** No file creation or replacement, no local protected-original claim, no journal phase transition, no S3 anchor or per-document baseline promotion.

This stage reuses the already merged provider-neutral plan_snapshot fast-forward rules and the fully verified current V1 S3 metadata store. It rejects partial/selective V2 advisory observations, foreign VaultId, wrong profile or baseline, missing/unknown ancestry, stale selected revision, siblings, tombstone/rename history for the candidate document and historic portable path collisions with any other document. Every known revision of the candidate must be Content at one identical path. Baseline and target content digests must agree with the verified DAG and immutable S3 blob key.

The returned snapshot ID, prior revision/hash and next revision/hash are an advisory proposal for the next stage, not a receipt or File Gate permit. Metadata verification does not guarantee the remote mutable HEAD/ETag remains unchanged, that the old blob is still available, or that the local original remains on disk. Native mutation requires a fresh S3 pin, staged immutable content, exact local original preservation, locked Core readback and OS-specific durability receipt. All missing proof remains HOLD. No automatic retry, conflict merge, last-writer-wins or deletion is introduced.

CI must run the new guarded_update_plan::tests under --features sync-s3 alongside the S3/MinIO suite; Core, docs and Main PR gate also required. This work is independent of P0a PR #300 and may be merged separately after exact-head CI.
