# SYNC-COORD-02f — real MinIO 10k offline V1/V2 discovery A/B

**Scope:** an ignored, isolated, two-AWS-SDK-client benchmark, plus a separately triggered GitHub Actions workflow. This is not an active production sync mode, File Gate gate, automatic publisher, writer-authentication mechanism or data-migration plan.

## Motivation

Merged #233 measured exact inventory parity on three revisions and merged #236 built the advisory selective V1-shard + V2-pack reader. The two-offline-generation MinIO smoke showed 11 GET / 22 HEAD for selective packs versus 24 GET / 48 HEAD for conservative V2 full re-pinning. A small fixture is not evidence of scalable cost or safe user-file application.

## Fixture

- **10,000 distinct Markdown documents** with one independently stored immutable V1 revision JSON and one content blob per document; 157 initial V1 shards (at most 64 refs per shard) and a single canonical generation-1 current root, all in a *fresh randomized disposable loopback MinIO bucket*.
- Three workloads: 0 changes (no invented generation or V2 envelope), 7 new revisions split 3 + 4 over two CAS-published generations, and 1,000 revisions split 500 + 500. Changes preserve all prior UUIDs and parent edges. Generation 2 adds an offline content sibling for document 0; generation 3 adds an independent tombstone sibling referencing the same old parent, with all other tombstones on distinct documents.
- Incremental manifest fixture preserves unmodified immutable shard IDs and digests; touched 64-entry shards are rebuilt and verified with up to 128 refs per shard. This is a **test partitioning policy**, not a deployed V1 sharding change, and its write amplification is measured.
- Pack builder exists only under `#[cfg(test)]`; individual V1 objects and V2 sidecar packs/letters are simultaneously stored to allow comparisons. Each pack has at most 128 refs and 512 KiB. Each envelope allows at most 1,024 refs; total chain capped at 8 generations and 16 MiB packs.
- Both clients use the official AWS Rust SDK and the same real profile-bound durable SQLite V1 anchor. All benchmark reads are read-only; neither can advance the anchor or write any local vault content.

## Assurance and modes

Cold source-of-truth verification (metadata materialization, full pinned snapshot and independently verified head) is measured separately and **not excluded from cold-start analysis**. After publishing generations 2 and 3, the benchmark compares the same remote state with:

1. **Selective V2 packs (02e):** V1 anchor + all V1 immutable ancestors + current/previous shard inventories and packs, but old blob present-day availability is NOT re-proven.
2. **Existing selective V1:** V1 anchor + changed shards, changed individual revision JSON/new blobs; previous immutable cache reused.
3. **Conservative V2 chain (02c):** all intermediate V1 roots fully pin every existing JSON and blob again, plus V2 letters/packs.
4. **Full final V1:** current complete immutable manifest and current blob-availability verifier. This only validates final state, **not** intermediary generation retention on its own.

Every mode must agree with the exact newly added UUID set, independent complete final V1 inventory, and preserve both content sibling and tombstone. The V2 selective reader must reject a missing or reversed letter. An altered final root/ETag, deleted tombstone, corrupt pack, missing blob or forged cache remains a HOLD: prior dedicated real-MinIO adversarial tests from #229/#236 remain mandatory.

0 edits is measured explicitly as a stable head with zero changes using the existing selective V1 path and full current pin. There is no V2 letter, so there is no legitimate V2-pack measurement for this workload.

## Outputs and interpretation

The `AKULA_COORD_02F_SCALE` single JSON row per job includes: workload, cold pin cost, genesis/changed-object publication cost (wall time), new immutable V1 revision/blob/shard counts and V2 pack count/bytes, per-mode attempted SDK GET/HEAD count, received bytes and elapsed microseconds, revision-count parity and unmodified durable anchor. Raw SDK method attempts are not TCP packets, retries or cloud provider billing; `AWS_MAX_ATTEMPTS=1` limits retries in CI.

Use `.github/workflows/sync-coord-02f-scale.yml` to reproduce the isolated MinIO workload. Its pull-request smoke initially uses 64 notes/7 changes; after smoke compilation and correctness pass, add the 10k 0/7/1000 matrix and require all four artifacts before accepting this stage. The expensive matrix is intentionally not added to every normal application change.

No performance comparison here authorizes user-file application. Old blob availability, remote writer authenticity/anti-rollback, capability negotiation, E2EE and safe File Gate crash handling remain independent requirements. A smaller S3 read count on immutable metadata is **not** complete synchronization.
