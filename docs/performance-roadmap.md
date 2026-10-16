# Akula performance roadmap

This document tracks measured performance work for the **Lalalalendia/Akula** fork. Follow the actual PR and Actions state; the checkboxes describe the plan as of 2026-10-16.

## Release rules
1. Each performance change is isolated in its own PR.
2. Capture a reproducible baseline **before** changing implementation code.
3. Use deterministic corpora and store raw CI logs, runner metadata and final commit SHAs.
4. Full `cargo test --workspace --locked` must pass on the final PR head.
5. Validate **correctness**, including user-observable behavior, before optimizing for speed.
6. Compare medians (multiple trials); do not claim end-to-end speedup from a local microbenchmark.
7. Merge one PR at a time, followed by main-branch verification. Keep upstream contributions separate.

## Work queue

| Step | Work | PR / evidence | Acceptance |
| --- | --- | --- | --- |
| 1 | Inline Markdown metadata fast path | [PR #4](https://github.com/Lalalalendia/Akula/pull/4), **merged** | Behavior-preserving tests; isolated parser benchmark ~5.8x faster; no claimed whole-app improvement |
| 2 | Core Performance CI and deterministic SQLite benchmark | [PR #6](https://github.com/Lalalalendia/Akula/pull/6), **merged**; paired runner [PR #11](https://github.com/Lalalalendia/Akula/pull/11), **merged** | Release-mode index, search, link, SQLite benchmark logs and runner details as artifacts |
| 3 | SQLite cached metadata inserts | [PR #7](https://github.com/Lalalalendia/Akula/pull/7), **closed without merge** | Same-run benchmark shows no improvement; no schema changes shipped |
| 4 | Correct backlinks count beyond 200 links | [PR #10](https://github.com/Lalalalendia/Akula/pull/10), **merged** | 245 distinct sources counted correctly; list UI remains capped at 200 |
| 5 | Parallel metadata preparation in indexer | [PR #9](https://github.com/Lalalalendia/Akula/pull/9) **merged safety fix**, [PR #14](https://github.com/Lalalalendia/Akula/pull/14) **closed without merge** | Batch-style worker pre-parsing slowed cold scans; only retry with a separately profiled streaming pipeline that overlaps parsing and writes |
| 6 | JPEG/RGBA decode optimization | [PR #12](https://github.com/Lalalalendia/Akula/pull/12), **closed without merge** | Direct JPEG decode was 31% slower on the controlled fixture; isolated RGBA scanline work remains unproven |
| 7 | Three-way merge / LCS | Extract from PR #2 selectively | Text-edit and UTF-16 correctness, no conflict regressions, statistically credible performance gain |
| 8 | Damage Tracker / frame allocations | Not started | Real UI frame timings and visual diffs, no dropped redraws |
| 9 | Clean up exploratory PRs | Mixed [PR #2](https://github.com/Lalalalendia/Akula/pull/2) and control [PR #3](https://github.com/Lalalalendia/Akula/pull/3) both **closed without merge** | Untested merge/LCS tweak remains inspectable in old PR #2; do not mix with separately validated changes |

## Active and completed benchmark work

- [PR #15](https://github.com/Lalalalendia/Akula/pull/15), **merged**: `AQUILUM_BENCH_RICH=1` generates 32 inline metadata fields, 32 tasks and 32 wiki links per note. The matched-run workflow uses the same rich corpus in both versions only after this PR merged into main.
- [PR #14](https://github.com/Lalalalendia/Akula/pull/14), **closed without merge**: [paired ABBA test](https://github.com/Lalalalendia/Akula/actions/runs/38950569881), AMD EPYC 7763, normal 1k-note cold scan 460.0 → 474.5 ms (+3.2%), rich 1k-note cold scan 787 → 850 ms (+8.0%). No convincing benefit; overhead of batch-barrier preparation likely dominates, but has not been conclusively profiled.
- [PR #16](https://github.com/Lalalalendia/Akula/pull/16), **merged**: skip the costly Markdown AST pass for documents without `[[`; [isolated parser benchmark](https://github.com/Lalalalendia/Akula/actions/runs/38950375959) 1.093 → 0.009 ms per 2.5k-paragraph note. This measures only wiki-parsing without links; no whole-index speedup claim.
- [PR #17](https://github.com/Lalalalendia/Akula/pull/17), **merged**: exercise the pre-existing `benchmark_save_breakdown` via GitHub Actions to profile document updates, Tantivy commit/reload, SQLite commit, metadata maintenance and fsync.


## Existing measurements

- [Control run](https://github.com/Lalalalendia/Akula/actions/runs/38934576531) and [combined optimized run](https://github.com/Lalalalendia/Akula/actions/runs/38934592230): full 512-note scan 259.7 → 215.6 ms; this bundles several optimizations and does **not** isolate their effects.
- [Isolated inline field run](https://github.com/Lalalalendia/Akula/actions/runs/38936592457): 8,000-line metadata parser 2.770 → 0.480 ms; cold full indexing shows no convincing overall change.
- First [Core Performance baseline](https://github.com/Lalalalendia/Akula/actions/runs/38940511422) (1,000-note run): cold 443 ms, warm 3 ms, update ten files 25 ms, delete ten files 10 ms. 512-note SQLite metadata benchmark median 44.773 ms.

## Matched-run measurement correction

The original SQLite control (44.773 ms) and optimized (15.915 ms) were run on **different CPUs** (AMD EPYC 7763 versus EPYC 9V45), so their ratio was misleading.

- [Paired control-workflow sanity test](https://github.com/Lalalalendia/Akula/actions/runs/38942500320): unchanged product code differed by 0.8% for SQLite.
- [Paired SQLite PR #7 comparison](https://github.com/Lalalalendia/Akula/actions/runs/38943049194), same AMD EPYC 9V74 in ABBA order: metadata 25.771 ms base versus 26.215 ms patched (+1.7% elapsed time); full indexing 347 ms both versions. **Caching was not merged**.
- The paired workflow is permanently available as `.github/workflows/core-perf-compare.yml`.

## Per-save hot path (not a global performance claim)

[Core Performance CI with the existing save breakdown test](https://github.com/Lalalalendia/Akula/actions/runs/38951047812) showed on its 1,000-note fixture:
- per-document index application: **0.29 ms**
- Tantivy `IndexWriter::commit`: **12.52 ms**
- index reload: **0.65 ms**
- SQLite commit: **0.30 ms**
- metadata maintenance: **0.54 ms**
- a separate 4 KiB `fsync` sample: **0.30 ms**

These are separately timed median sub-operations, not an additive breakdown of a specific full user save. In `search/worker.rs`, filesystem events are already coalesced over **80 ms**, and the Tantivy writer is released after **5 seconds** idle. Do not add a second debounce or defer index freshness without first measuring user-visible behavior; investigate Tantivy commit cost and the trade-offs of repeated versus batched commits.

## Benchmark commands

From `aquilum-app`:

```sh
cargo test --workspace --locked
AQUILUM_BENCH_SIZES=1000 cargo test --release --locked -p aquilum-core benchmark_indexing -- --ignored --nocapture --test-threads=1
AQUILUM_BENCH_SIZES=1000 cargo test --release --locked -p aquilum-core benchmark_search_pipeline -- --ignored --nocapture --test-threads=1
AQUILUM_BENCH_SIZES=1000 cargo test --release --locked -p aquilum-core benchmark_link_pipeline -- --ignored --nocapture --test-threads=1
cargo test --release --locked -p aquilum-core benchmark_sqlite_metadata -- --ignored --nocapture --test-threads=1
cargo test --release --locked -p aquilum-core benchmark_save_breakdown -- --ignored --nocapture --test-threads=1
AQUILUM_BENCH_RICH=1 AQUILUM_BENCH_SIZES=1000 cargo test --release --locked -p aquilum-core benchmark_indexing -- --ignored --nocapture --test-threads=1
```

The SQLite-specific benchmark and both CI workflows are now available in `main`. Compare exact heads on the same CPU; runner-to-runner numbers alone are not sufficient.
