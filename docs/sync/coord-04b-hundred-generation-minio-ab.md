# SYNC-COORD-04b — 100 offline generations, real S3/MinIO read-only A/B

Status: **test-only A/B fixture based on accepted #275**. No changes to production sync reader, published wire format, files or checkpoint.

## Purpose

The new opt-in long-offline reader in #275 can verify 9–128 missed V1 generations. This lab measures whether its two complete V1 ancestry walks, one locator per generation, one immutable delta letter/pack per generation and reused shard inventory are still competitive when a client misses **100 generations**.

It is **not** correct to claim that a 100-generation V2 replay is faster simply because two-generation #238 was faster. The full V1 pin becomes relatively cheaper if all 100 new revisions are small; this test may show V2 more expensive. The outcome must influence whether the future read-only recovery selector chooses V2 or full repin.

## Fixture and invariants

The ignored real MinIO integration test `live_minio_v2_hundred_offline_generations_vs_full_v1_ab` uses a disposable random bucket and two independent AWS-SDK clients:

1. One fully pinned genesis Markdown revision, a profile-bound SQLite remote anchor and trusted metadata cache.
2. One hundred sequential V1 immutable manifest generations and conditional S3 head CAS operations, with 100 V2 immutable packs, letters, and locator hints. Every new RevisionRecord has the same original parent (one tombstone), deliberately preserving 100 siblings/conflict candidates.
3. The existing eight-generation public V2 discovery must refuse. The new opt-in long reader must report **100 verified generations, 100 packs, 101 revision IDs**, all exact IDs, and `requires_full_apply_gate=true`.
4. Fresh full V1 pin on exactly the same final head; compare 101-item inventory including original and all 100 siblings. The original SQLite anchor must still be genesis.
5. Record separate SDK-attempt GET and HEAD, completely read payload bytes, and elapsed wall time for the new V2 selective verifier and fresh full V1 pin. These are **two different assurance classes**: V2 reuses previously pinned old-blob evidence while V1 reproves current old-blob availability.

CI: existing `.github/workflows/sync-s3-minio.yml` runs ignored `live_minio_` tests on localhost, and prints `AKULA_COORD_04B_LONG_OFFLINE` JSON. Do not report specific results before the run is green.

## Interpretation and next decisions

If 100-generation V2 costs more SDK calls or more wall time than full V1 on this tiny per-generation fixture, **do not conceal the result**. The right next optimization may be to eliminate the duplicated V1 walk, batch immutable requests with bounded concurrency, or select an independently verified full snapshot fallback in the special expensive case. Remote publisher authority, old data retention, ACL/custody, local File Gate apply, S3 delete/GC, baseline promotion and anti-rollback remain outside both test paths.

A future more realistic matrix should also compare 0/7/1000 edits across 20/100 generations with 10,000 existing files and WAN RTT. This specific test prioritizes history retention and per-generation request overhead and MUST NOT be called an end-to-end sync speed benchmark.

## Measured real localhost MinIO (exact #279 A/B CI run)

Source: [green MinIO run 39140096004](https://github.com/Lalalalendia/Akula/actions/runs/39140096004/job/117712562369), head `3012314c0f61ab61f7be1f288a4970a5989514d4`; see [PR #279 results](https://github.com/Lalalalendia/Akula/pull/279#issuecomment-6251660499). Raw fixture results:

| Missed generations | Read mode | SDK GET attempts | SDK HEAD attempts | Fully read payload bytes | Wall time |
|---:|---|---:|---:|---:|---:|
| 20 | V2 selective | 125 | 250 | 106,369 | 485 ms |
| 20 | Full V1 | 90 | 180 | 88,519 | 294 ms |
| 100 | V2 selective | 605 | 1,210 | 1,362,992 | 2,502 ms |
| 100 | Full V1 | 410 | 820 | 1,264,574 | 1,329 ms |

At 100 tiny per-generation changes V2 required 47.6% more GET/HEAD attempts and took 1.88x the wall time. Both paths produced the same 101 revision IDs, including the tombstone and siblings; the profile-bound SQLite checkpoint remained at genesis. The old eight-generation entrypoint still refused. The modes have **different assurance guarantees** (V2 reuses historical old-blob evidence; full V1 checks present-day old-blob availability), so this is **not** evidence that one may silently replace the other in production.

**Engineering next step:** remove duplicate verified V1 ancestry walks (without dropping final-head stability, historical-link or shard coverage checks), then re-run paired tests. The local MinIO timing is not WAN/10k-file performance.
