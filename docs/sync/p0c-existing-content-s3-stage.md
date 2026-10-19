# SYNC-APPLY-P0c — verified registered existing-file S3 stage

Status: **draft, stacked on #303**, read-only user vault. Adds an independent Core path for a fully verified existing-content fast-forward; the existing additive-only stage retains its strict expected_baseline=None gate.

Admission: registered root, approved S3 profile and VaultId, journal/cache SQLite locations outside vault, current persisted ConfirmedBaseline and previous committed operation at the same path/document/hash. Use full canonical verified S3 V1 DAG and P0b read-only fast-forward planner; no V2 advisory preview or last-writer-wins. Pin S3 head and ETag, download immutable bounded BLAKE3 content and recheck S3 head plus host baseline before and after private appdata WAL/FULL stage.

**No user file modification, native preservation receipt, StageDownload Applied/Verified/Committed transition, checkpoint promotion, replay after ambiguous failure, or S3 observation anchor movement.** A later native OS File Gate replace executor with recoverable original, pinned OS IDs and native mutation receipt is mandatory.

Acceptance requires actual loopback MinIO old-baseline/new-content integration with unchanged user file and unchanged confirmed baseline, wrong profile/path/head refusals, and exact-head Rust/docs/Main gates. Do not merge without the integration fixture and green CI.
