# STAYA YDB-REPL-01 — reuse verified LesHa replication algorithms without importing storage authority

**Scope:** isolated research in Akula. No main modifications, File Gate writes, network connections or product-level receipt authority. Source: [LesHa replication](https://github.com/Lalalalendia/med/blob/5046ff53207ce2fc89fb83277d9ceec1fee30935/crates/lesha-replication/src/lib.rs), pinned exact SHA `5046ff53207ce2fc89fb83277d9ceec1fee30935`. Context: [YDB quorum tracker](https://github.com/ydb-platform/ydb/blob/6951d3dba563c6d775a689422d87b4c7febabd51/ydb/core/blobstorage/dsproxy/dsproxy_quorum_tracker.h).

## Objective

A/B/C have three full independent verified copies. Losing A leaves B/C healthy but policy degraded. Repair planner selects independently placed D within hard limits. D becomes a counted third replica **only** after its simulated local commit, exact SHA-256 byte readback, identity/generation check, signature validation and admission to the pure LesHa evaluator.

The test-only receipt gate uses Ed25519 and a one-process synthetic disk. It **does not** prove remote fsync or actual possession; because the mock runs in one process and the gate can inspect its bytes, it deliberately does not claim to be production-ready. The project's real Akula content/RevisionId format may use a different digest/identity and must not be cast to LesHa `ChunkId` merely because both are 32 bytes.

## Positive and negative cases

- Three verified independent host/device placements satisfy `ReplicaPlacementPolicy`; two copies on same host do not.
- A disappears; `plan_bounded_repairs` selects D over a candidate sharing B's host. `projected.state` must not be treated as actual durability.
- Before simulated commit, even a valid signed `declared_committed=true` receipt is rejected; after simulated commit + readback it may be admitted.
- Corrupt B's bytes; hash verification rejects B's new receipt, physical scrub observation removes health credit; D must be populated from intact C.
- Different storage generation, device enrollment generation, subject hash, receipt signature, missing/duplicate receipt reject or fail to increase durability credit.
- Strict max_jobs/max_bytes keep repair deferred rather than pretending a copy exists.

## Critical limitations

1. `VerifiedReplicaReceipt::new_prevalidated()` in LesHa is a **pure trust-boundary constructor**, not a signature verifier. Do not expose it to unverified peer messages.
2. Synthetic `committed` flag is **not** `fsync`, not persisted and not physical fault isolation. Never publish `Replicated` in product on this basis.
3. The LesHa source is in repository `Lalalalendia/med` with root `UNLICENSED`; license and ownership must be resolved before depending on this library in distributed production artifacts.
4. Akula File Gate and Sync Core remain authoritative for actual files, grants, canonical revision graph and writes. No additional Writer is introduced.
5. Next step after GREEN: a real trusted receipt adapter and crash/power-loss/restart test, then actual Roha/noq link between physically separate A/B/C/D.

## Run

```sh
git clone https://github.com/Lalalalendia/med deps
git -C deps checkout 5046ff53207ce2fc89fb83277d9ceec1fee30935
cargo generate-lockfile --manifest-path experiments/staya-ydb-repl-01/Cargo.toml
cargo test --locked --manifest-path experiments/staya-ydb-repl-01/Cargo.toml --all-targets -- --nocapture
```

The linked GitHub Actions workflow checks out the exact source itself and emits test logs. **No test result is claimed until the current CI run completes.**
