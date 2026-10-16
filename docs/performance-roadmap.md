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
| 5 | Parallel metadata preparation in indexer | [PR #9](https://github.com/Lalalalendia/Akula/pull/9), **merged prerequisite**; next: extract from [PR #2](https://github.com/Lalalalendia/Akula/pull/2) | Worker panics now surface as errors; next: bounded workers, cancellation, identical indexed content |
| 6 | JPEG/RGBA decode optimization | [PR #12](https://github.com/Lalalalendia/Akula/pull/12), **Draft** | Pixel/dimension equivalence, cross-platform CI and representative decode benchmarks before merge |
| 7 | Three-way merge / LCS | Extract from PR #2 selectively | Text-edit and UTF-16 correctness, no conflict regressions, statistically credible performance gain |
| 8 | Damage Tracker / frame allocations | Not started | Real UI frame timings and visual diffs, no dropped redraws |
| 9 | Clean up exploratory PRs | [PR #2](https://github.com/Lalalalendia/Akula/pull/2), [PR #3](https://github.com/Lalalalendia/Akula/pull/3) | Close only after useful portions and baselines have been preserved |

## Existing measurements

- [Control run](https://github.com/Lalalalendia/Akula/actions/runs/38934576531) and [combined optimized run](https://github.com/Lalalalendia/Akula/actions/runs/38934592230): full 512-note scan 259.7 → 215.6 ms; this bundles several optimizations and does **not** isolate their effects.
- [Isolated inline field run](https://github.com/Lalalalendia/Akula/actions/runs/38936592457): 8,000-line metadata parser 2.770 → 0.480 ms; cold full indexing shows no convincing overall change.
- First [Core Performance baseline](https://github.com/Lalalalendia/Akula/actions/runs/38940511422) (1,000-note run): cold 443 ms, warm 3 ms, update ten files 25 ms, delete ten files 10 ms. 512-note SQLite metadata benchmark median 44.773 ms.

## Matched-run measurement correction

The original SQLite control (44.773 ms) and optimized (15.915 ms) were run on **different CPUs** (AMD EPYC 7763 versus EPYC 9V45), so their ratio was misleading.

- [Paired control-workflow sanity test](https://github.com/Lalalalendia/Akula/actions/runs/38942500320): unchanged product code differed by 0.8% for SQLite.
- [Paired SQLite PR #7 comparison](https://github.com/Lalalalendia/Akula/actions/runs/38943049194), same AMD EPYC 9V74 in ABBA order: metadata 25.771 ms base versus 26.215 ms patched (+1.7% elapsed time); full indexing 347 ms both versions. **Caching was not merged**.
- The paired workflow is permanently available as `.github/workflows/core-perf-compare.yml`.

## Benchmark commands

From `aquilum-app`:

```sh
cargo test --workspace --locked
AQUILUM_BENCH_SIZES=1000 cargo test --release --locked -p aquilum-core benchmark_indexing -- --ignored --nocapture --test-threads=1
AQUILUM_BENCH_SIZES=1000 cargo test --release --locked -p aquilum-core benchmark_search_pipeline -- --ignored --nocapture --test-threads=1
AQUILUM_BENCH_SIZES=1000 cargo test --release --locked -p aquilum-core benchmark_link_pipeline -- --ignored --nocapture --test-threads=1
cargo test --release --locked -p aquilum-core benchmark_sqlite_metadata -- --ignored --nocapture --test-threads=1
```

The SQLite-specific benchmark and both CI workflows are now available in `main`. Compare exact heads on the same CPU; runner-to-runner numbers alone are not sufficient.
