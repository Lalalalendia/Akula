# SYNC-DELTA-01h/01j — full, warm and cold-cache S3/MinIO A/B

**Status:** warm-vs-full [10k-note run #39075239066](https://github.com/Lalalalendia/Akula/actions/runs/39075239066) was completed (three green jobs, 0/7/1000 edits). PR #188 added strictly read-only, profile-bound persisted metadata hints. This revision adds a **third measured cold-SQLite-cache phase** to the same real MinIO experiment. No new cold measurements should be claimed until the revised workflow actually runs and publishes three-row artifacts.

## Actual workloads

- 10,000 distinct Markdown notes, each represented by one content blob, an immutable JSON revision, and sharded `S3ManifestV1` roots (up to 128 references per shard).
- Three separately isolated runs: unchanged root (**0** edits), **7** edited notes, and **1,000** edited notes. Changed notes are evenly distributed over the 10,000 existing documents, providing a deterministic scattered-ID adversarial case. Revision IDs and per-document parents are deterministic; blobs are unique for each note and generation.
- Initial state is uploaded into ephemeral localhost MinIO using at most 32 concurrent conditional PUTs. Baseline revision DAG and blobs are fully verified by the canonical `materialize_verified_revision_metadata_store` and `read_verified_head`/`pin_revision_snapshot`. The private SQLite anchor **and the separate metadata hint-cache journal** are then persisted and reopened. This full verification, upload and SQLite-save setup is excluded from timed comparisons.
- The one-generation successor (7/1,000 changes) keeps exact old shard IDs where possible and rewrites only affected shard ranges, using new immutable shard objects and a server-side CAS update of `refs/current.json`. This is a **benchmark fixture publishing policy**, not a change to production sharding, remote protocol or sync privileges.
- **Phase A (warm):** in-memory verified cache `inspect_selective_s3_revision_changes`. **Phase B (cold metadata):** discard the RAM cache and SDK client, recreate client with a fresh HTTP connection pool, reopen private SQLite anchor and hint cache, call `inspect_selective_from_persisted_hints`. Measure SQLite restoration and new-only S3 reads together (connection creation excluded). **Phase C (full):** current full `pin_revision_snapshot` on the same root. Both selective phases return **advisory previews only**, and must download exactly 0/7/1,000 new revision JSONs and blobs; full phase freshly checks old-object availability. No remote writes happen between measured phases.
- Each case emits **three** JSONL records with `scenario` = `selective_preview`, `cold_disk_selective_preview` or `full_verified_pin`; includes `notes`, `new_revisions`, `elapsed_ms`, attempted S3 HEAD/GET, received payload bytes and distinct security-scope labels. Actual numbers must be read from the finished GitHub Action/artifacts, never extrapolated from the historical 10k warm-cache run.

## Important measurement limits

- Test-only `Arc<AtomicU64>` instrumentation counts Akula calls to the real AWS SDK's HEAD/GET methods before network sends, plus bytes successfully received in bounded GET/stream digest paths. It excludes **SDK-internal automatic retries**, HTTP/TLS protocol overhead, TCP retransmissions and overhead from uploads and initial full verification. The workflow sets `AWS_MAX_ATTEMPTS=1` to avoid accidental retries in MinIO.
- **Not equivalent security guarantees:** selective is a *change-discovery advisory preview* that reuses previously verified immutable objects. It does not establish their continued current availability or permission to mutate files. Full `pin_revision_snapshot` performs fresh integrity and availability checks on all referenced blobs and ancestors. A measured ratio is **NOT** an end-to-end sync speedup or a recommendation to disable full File Gate verification.
- Phase order is warm preview → cold SQLite restoration + preview with **new AWS SDK client** → full pin. This controls the remote root but does **not** counterbalance data-center cache/TLS/service warmth; therefore real wall times require repeatable runs and WAN RTT injection before product-level claims. Cold measured time excludes process launch and new SDK connection construction, but **includes** private SQLite read/row hashing/DAG reconstruction and selective S3 requests.
- A single live MinIO test uses localhost and no WAN RTT. The profile supports HTTPS S3, but latency scenarios 1/30/100/200ms are separate experiments, not produced here.
- The cold recovery source is the private SQLite metadata hints implemented by [PR #188](https://github.com/Lalalalendia/Akula/pull/188), not a fresh full S3 pin. Its original **full cold verification before persistence** is excluded, so a brand-new device with no cache can still be expensive. Both warm and persisted caches remain hints and do not prove that old S3 blobs exist today. `S3ManifestV1` still supports at most **32,768** revision references.
- A scenario where the publisher completely re-chunks sorted UUIDs rather than stabilizing ranges can rewrite far more shards; keep this alternative as an independent benchmark case and do not overstate improvements.

## How to run and where results are stored

### PR smoke gate (automatic)

`.github/workflows/sync-s3-delta-ab.yml` runs a single small real MinIO smoke workload, **64 notes + 7 revisions**, whenever benchmark/transport/manifest source changes. This proves the harness compiles, uses actual S3 and emits **three** parseable JSON rows. It does **not** substitute for the 10,000-note run.

### Complete study (GitHub Actions, automatic on workflow-file update or dispatch)

A merge **modifying this benchmark workflow YAML file in main** triggers the full 10k study (push event restricted to that file), without burdening unrelated PRs. Alternatively, use GitHub Actions → **S3 delta A/B network benchmark** → **Run workflow**. One full push/dispatch creates three parallel, individually isolated disposable MinIO jobs: **10,000 notes × 0, 7, 1,000 edits**. Each job builds and fully verifies the initial vault, publishes the chosen successor, then records warm selective, persisted SQLite cold selective and full fresh-pin metrics separately.

Artifacts: `akula-delta-ab-push-{0,7,1000}` (automatic first merge) or `akula-delta-ab-workflow_dispatch-{0,7,1000}` (manual), containing `delta-ab-results.jsonl` with **three rows** each. The job also prints lines beginning `AKULA_DELTA_AB` for log-based audit. Empty/incomplete JSON is a hard failure. The benchmark workflow itself does not issue any S3 DELETE/GC or touch user data; temporary MinIO runs in Docker tmpfs and is stopped in a guaranteed cleanup step.

The previous two-phase warm-vs-full 10k study is **complete** (see linked run). The **new three-phase cold-SQLite** 10k study remains unmeasured until the updated workflow completes and its artifacts are inspected.

## Next gates

1. Pass PR smoke + required Rust/Windows/macOS/Linux/MinIO CI on exact final head; merge only after green.
2. Execute the upgraded three-phase 10k A/B and record **actual cold persisted-cache numbers**, request counts, downloaded bytes, elapsed and environment for 0/7/1000 edits. Preserve the historical warm/full results as a distinct study.
3. Extend with randomized/shuffled UUID placement, contiguous 7-note edits, unstable-shard re-chunk publisher, WAN RTT and packet loss, and genuine end-to-end whole-app cold-process startup.
4. Review private SQLite metadata confidentiality and external writer/anti-rollback proofs, plus fresh remote availability and File Gate admission before considering selective data for actual file application. No such permissions are changed here.

References: [Notion SYNC-DELTA-01](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360); [Tela SYNC-DELTA-01](https://telawiki.com/spaces/456/pages/7501/sync-delta-01-shard-reuse); [PR #175](https://github.com/Lalalalendia/Akula/pull/175).
