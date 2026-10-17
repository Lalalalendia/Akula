# SYNC-COORD-01 — experimental coordinate Merkle tree + immutable delta letter + verified pack

**Stage:** CODED TEST-ONLY EXPERIMENT, not a production S3 v2 protocol. The trial adds no production writer, remote schema, background scheduler, File Gate apply or GC. All experimental data is written into a disposable localhost MinIO bucket under a random test VaultId. **No performance numbers should be claimed until GitHub Actions produces the `AKULA_COORD_AB` JSONL artifacts.**

## Context and hypothesis

The historic [10k-note SQLite-cold S3 benchmark](https://github.com/Lalalalendia/Akula/actions/runs/39077557170) found **33 AWS SDK GET + 66 HEAD** for 7 edits. These calls include verification of the mutable head, immutable roots, shards, revision JSON and blobs — **not 33 content downloads**. The user proposes recursively numbered cells `0–6`, a stable document coordinate, one short exchange of changed IDs and packing small altered files into a single immutable object.

In Alice Universe, [«Вертикальная карта реальности»](https://telawiki.com/spaces/309/pages/3878) distinguishes WORLD-ID, POSITION, ROUTE, REGION and LAYER/ANCHOR/DEPTH. This inspires separating document *identity*, logical *index position*, and data *transport*. It is **not** evidence that the Alice Universe wiki already specifies this 7-ary S3 protocol. Real H3 hexagon geometry is unnecessary for Markdown files; the test uses a purely logical BLAKE3 hash address, not physical spatial neighbors.

## Experiment mechanics

- Stable address: `BLAKE3(derive_key("akula/sync/coordinate-probe/v1"), vault UUID, document UUID)`, then first digits modulo branch factor `7`, `16` or `32`. Length 5/4/3 respectively; **collisions are represented as lists at a leaf, not overwritten**. A rename or new revision never relocates the document.
- Old/new Merkle trees contain complete *revision reference histories* for each document, **including parent/sibling/tombstone IDs**, not just current Markdown bodies. Per-node serialized JSON has BLAKE3 content hash; unchanged nodes reuse exact digests. Each changed subtree is retrieved from MinIO and independently re-hashed. Per-leaf algorithm fails closed on vanished document/history, reused UUID with altered ref, or duplicate revision.
- Experiment-only `ChangeLetter` carries `prior_seven_root_digest`, `vault_id`, exact changed `S3RevisionRefV1` list, test packed-object digests and the three new Merkle roots. Stored as one immutable S3 object, checked against a test-fixture-known digest and vault/previous-root identity. **This is not authenticated CAS or writer identity and is not connected to the current production S3 root.**
- Pack bytes use the **existing** `s3_transport/pack_probe.rs::PackBuilder` + `ingest_verified_pack` (already tested) without duplicating a custom archive. It enforces 128 immutable revision+blob pairs and a 512 KiB pack cap, checks whole-pack BLAKE3, every revision JSON, every content blob, document identity and DAG closure with the previously verified old records.
- The new `coord_probe.rs` is registered **only behind `#[cfg(test)]`** in the existing opt-in S3 transport. We change **zero production S3 wire keys or manifest/root schemas**; all new objects use already whitelisted immutable content-addressed `blobs/<blake3>` keys in the disposable fixture.

## Five A/B read modes on exactly the same changed notes

| Mode | Reads/validates |
| --- | --- |
| `letter_individual` | one immutable letter plus N individual revision JSON and content blob GETs |
| `letter_pack` | the same letter plus ceil(N/128) immutable packs, every member and DAG independently verified |
| `merkle_7_pack` | letter, only changed Merkle nodes of a logical 7-ary coordinate trie, then verified packs |
| `merkle_16_pack` | same, 16-ary trie |
| `merkle_32_pack` | same, 32-ary trie |

The old 10k Gen1 note histories are a **synthetic trusted local cache**, and only new immutable data + changed Merkle nodes are uploaded to the isolated MinIO experiment. The test does **not** use unanchored remote data as a new production full pin. It does not materialize existing 10k old remote Merkle nodes and cannot claim their current availability.

The test reports one machine-readable `AKULA_COORD_AB` line per method with:
- attempted AWS SDK `GET` and `HEAD`, received payload bytes, wall time;
- fixture publication overhead: count and payload bytes for immutable objects written, including shared new revision/blob objects, packs and letter, plus changed Merkle nodes;
- `letter_bytes` for index-overhead tradeoffs;
- explicit `assurance=test-only-trusted-fixture-read-only-no-file-gate`.

**No S3 PUT is included in measured read latency.** Request counters cover Akula's SDK attempted methods (HEAD/GET), not network/TCP bytes or automatic internal SDK retries; retries are disabled in MinIO CI. Real WAN RTT, upload wall time, packing CPU and whole-app startup need separate further trials.

## Test matrix and execution

- Automatic PR MinIO smoke: 64 notes, 7 edits. Must produce **exactly five valid JSONL rows**, shared workload, exact revision conservation, same count for every method, all Merkle diff sets matching the letter and verified packs. Metadata+payload pack GETs should be no more than individual downloads.
- After acceptance and merge of [`sync-coord-ab.yml`](https://github.com/Lalalalendia/Akula/blob/sync/coord-01-merkle-envelope-pack-ab-20261018/.github/workflows/sync-coord-ab.yml) into main, one automatic full matrix: 10k notes × **0/7/1000 edits** using separate disposable localhost MinIO per job; also can be `workflow_dispatch`ed. One artifact `akula-coord-ab-push-{0,7,1000}` per scenario, retaining five measurement rows.
- Pure Rust adversarial tests: stable identity address, serialization-independent order, missing tombstone, rewritten UUID, duplicate revision, wrong pack count, tampered letter. Existing [`pack_probe` tests](https://github.com/Lalalalendia/Akula/blob/main/aquilum-app/core/src/sync/s3_transport/pack_probe.rs) further cover altered blob/revision/length, siblings and DAG.

## Interpretation gates

- One letter + one pack **may** yield two content/index GETs for exactly 7 updates **in this test fixture**; it is emphatically **not** two requests for complete S3 synchronization. Production CAS head, permissions, ETag proof, conflict retention, manifest consistency and safe File Gate application still require their own I/O.
- Merkle paths can use **more reads** than a direct changed-ID letter. This experiment is intended to measure that tradeoff and answer whether 7, 16 or 32 are best, not predetermine the winner.
- Building and publishing many immutable Merkle nodes can introduce severe *write amplification*, especially with 1000 scattered edits. Watch **new node counts and published bytes**, not just read GET counts. Do not optimize with fake “unchanged since timestamp” claims — durable generations, history and revision DAG must be trusted.
- One missing old blob in S3 makes any advisory read insufficient for direct File Gate writes. Current [per-generation retention verification PR #197](https://github.com/Lalalalendia/Akula/pull/197) must not be bypassed. No current tests prove remote writers authenticated or local SQLite anti-rollback protected.
- If later implementing v2, negotiate capabilities and migrations and keep strict v1 `S3HeadPointerV1`, `S3ManifestV1` and `S3Keyspace::contains_key` fail-closed. Never slip new fields into strict `deny_unknown_fields` v1 blobs.

Source notes: [Tela SYNC-COORD-01](https://telawiki.com/spaces/456/pages/7518/sync-coord-01-merkle), [Notion SYNC-DELTA-01](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360).
