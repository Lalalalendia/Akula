# SYNC-DELTA-01h — measurable S3/MinIO A/B, 10,000 Markdown notes

**Status:** reproducible benchmark harness with measured application-level AWS SDK HEAD/GET *attempts*, successfully read payload bytes and elapsed wall time. No benchmark results should be claimed without a completed workflow log/artifact. This is **not** the old 10,000-object upload/listing test.

## Actual workloads

- 10,000 distinct Markdown notes, each represented by one content blob, an immutable JSON revision, and sharded `S3ManifestV1` roots (up to 128 references per shard).
- Three separately isolated runs: unchanged root (**0** edits), **7** edited notes, and **1,000** edited notes. Changed notes are evenly distributed over the 10,000 existing documents, providing a deterministic scattered-ID adversarial case. Revision IDs and per-document parents are deterministic; blobs are unique for each note and generation.
- Initial state is uploaded into an ephemeral localhost MinIO bucket using at most 32 concurrent bounded conditional PUTs. Baseline revision DAG and blobs are fully verified using the **existing** `materialize_verified_revision_metadata_store` and canonical `read_verified_head`/`pin_revision_snapshot`; only then is the profile-bound SQLite anchor persisted and reopened. Setup work is intentionally **outside** measured phases.
- The one-generation successor (7/1,000 changes) keeps exact old shard IDs where possible and rewrites only affected shard ranges, using new immutable shard objects and a server-side CAS update of `refs/current.json`. This is a **benchmark fixture publishing policy**, not a change to production sharding, remote protocol or sync privileges.
- Phase A: `inspect_selective_s3_revision_changes`, returning a read-only `S3SelectiveRevisionPreview`. Phase B: the existing full `pin_revision_snapshot` on **the same final head**. No S3 changes occur between phases. Assert both see the expected revision count and the selective path downloaded exactly 0/7/1,000 new JSON/blobs.
- The benchmark writes a machine-readable two-line JSON artifact for each run with `scenario`, `notes`, `new_revisions`, `elapsed_ms`, `attempted_s3_head`, `attempted_s3_get`, `downloaded_payload_bytes`, and `security_scope`. These are observed values, not estimates.

## Important measurement limits

- Test-only `Arc<AtomicU64>` instrumentation counts Akula calls to the real AWS SDK's HEAD/GET methods before network sends, plus bytes successfully received in bounded GET/stream digest paths. It excludes **SDK-internal automatic retries**, HTTP/TLS protocol overhead, TCP retransmissions and overhead from uploads and initial full verification. The workflow sets `AWS_MAX_ATTEMPTS=1` to avoid accidental retries in MinIO.
- **Not equivalent security guarantees:** selective is a *change-discovery advisory preview* that reuses previously verified immutable objects. It does not establish their continued current availability or permission to mutate files. Full `pin_revision_snapshot` performs fresh integrity and availability checks on all referenced blobs and ancestors. A measured ratio is **NOT** an end-to-end sync speedup or a recommendation to disable full File Gate verification.
- The default test performs selective first, then full, which may change cache warmth. Both phases report exact process wall time, but repeated/counterbalanced remote runs and RTT injection are still needed before product-level latency claims.
- A single live MinIO test uses localhost and no WAN RTT. The profile supports HTTPS S3, but latency scenarios 1/30/100/200ms are separate experiments, not produced here.
- The cache is **in memory** and was acquired via expensive cold pinning; cold-start performance after the app restarts remains unresolved. `S3ManifestV1` still supports at most **32,768** revision references.
- A scenario where the publisher completely re-chunks sorted UUIDs rather than stabilizing ranges can rewrite far more shards; keep this alternative as an independent benchmark case and do not overstate improvements.

## How to run and where results are stored

### PR smoke gate (automatic)

`.github/workflows/sync-s3-delta-ab.yml` runs a single small real MinIO smoke workload, **64 notes + 7 revisions**, whenever benchmark/transport/manifest source changes. This proves the harness itself compiles, uses actual S3 and emits two parseable JSON measurement rows. It does **not** substitute for the 10,000-note run.

### Complete study (manual GitHub Actions)

Use GitHub Actions → **S3 delta A/B network benchmark** → **Run workflow** on a revision containing the benchmark. One workflow dispatch creates three parallel, individually isolated disposable MinIO jobs: **10,000 notes × 0, 7, 1,000 edits**. Each job builds and fully verifies the initial vault, publishes the chosen successor, then records selective/full read metrics separately.

Artifacts: `akula-delta-ab-workflow_dispatch-0`, `...-7`, `...-1000`, containing `delta-ab-results.jsonl` with two rows each. The job also prints lines beginning `AKULA_DELTA_AB` for log-based audit. Empty/incomplete JSON is a hard failure. The benchmark workflow itself does not issue any S3 DELETE/GC or touch user data; temporary MinIO runs in Docker tmpfs and is stopped in a guaranteed cleanup step.

No background work is implied by this document: until a workflow dispatch is actually executed and succeeds, the 10k measurements remain **unrun**.

## Next gates

1. Pass PR smoke + required Rust/Windows/macOS/Linux/MinIO CI on exact final head; merge only after green.
2. Execute the full dispatched 10k A/B and record **actual numeric results**, requests, received bytes, elapsed and environment; compare performance by edit count, not model extrapolation.
3. Extend with randomized/shuffled UUID placement, contiguous 7-note edits, unstable-shard re-chunk publisher, WAN RTT and packet loss, and a cold app restart.
4. Design durable verified metadata cache and a revised complete safety proof before using selective data as File Gate write authority.

References: [Notion SYNC-DELTA-01](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360); [Tela SYNC-DELTA-01](https://telawiki.com/spaces/456/pages/7501/sync-delta-01-shard-reuse); [PR #175](https://github.com/Lalalalendia/Akula/pull/175).
