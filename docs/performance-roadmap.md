# Akula performance roadmap

This document tracks measured performance work for the **Lalalalendia/Akula** fork. Follow the actual PR and Actions state; the checkboxes describe the plan as of 2026-10-17.

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

These are separately timed median sub-operations, not an additive breakdown of a specific full user save. At the time of this measurement, `search/worker.rs` used a fixed **80 ms** event-collection window; [PR #28](https://github.com/Lalalalendia/Akula/pull/28) has since added adaptive collection while keeping an 80 ms maximum. The Tantivy writer is released after **5 seconds** idle. Do not add a second debounce or defer index freshness without first measuring user-visible behavior; investigate Tantivy commit cost and the trade-offs of repeated versus batched commits.

## Tantivy commit follow-up (2026-10-17)

The initial save benchmark used Tantivy's default writer while production uses `writer_with_num_threads(2, 40 MB)`. That discrepancy is corrected in [PR #21](https://github.com/Lalalalendia/Akula/pull/21), **merged** (no runtime behavior change).

- [Production-writer benchmark](https://github.com/Lalalalendia/Akula/actions/runs/38955964260), AMD EPYC 7763: `tantivy_commit_ms=11.91`, including `prepare_commit` segment flush **9.66 ms** and final durable `PreparedCommit::commit` **2.07 ms**; reload **0.64 ms**. These phase medians are measured individually and need not sum exactly.
- [Thread-count ABBA experiment](https://github.com/Lalalalendia/Akula/actions/runs/38955409107), [PR #20](https://github.com/Lalalalendia/Akula/pull/20), **closed without merge**: two-thread commits 11.800/11.613 ms versus one-thread 11.472/12.267 ms, with search visibility checked after every commit. No consistent win from changing the production writer thread count.
- [PR #23](https://github.com/Lalalalendia/Akula/pull/23), **merged**: avoid a needless Tantivy commit only for deleted `.md` files absent from **both** metadata and the live index, while retaining orphan cleanup. [Same-runner benchmark](https://github.com/Lalalalendia/Akula/actions/runs/38957006845) for never-indexed deletion events on a populated 512-note index: previous behavior **0.869 ms**, patched path **0.014 ms** per event (median, ABBA). This does **not** speed up real document commits; full Rust CI and targeted ABBA benchmark passed before merge.

Future Tantivy work should focus on measuring the **segment preparation/flush phase**, since it dominates real replacement commits. Do not suppress or defer commits for indexed document edits merely to improve a benchmark: immediately searchable results and crash durability remain required.

## Commit batch-scaling experiment and worker-batch follow-up (2026-10-17)

- [PR #25](https://github.com/Lalalalendia/Akula/pull/25), **merged, test-only**; [GitHub Actions](https://github.com/Lalalalendia/Akula/actions/runs/38961681206) passed Rust CI, core benchmark and search/durability checks. The 512-note fixture updated **64 notes** via production `apply_paths` in ABCCBA order on AMD EPYC 7763 (4 vCPUs):

  | Group size | Total commits | Total time over 64 notes (two runs) | Median total | Median per edited note |
  | --- | ---: | ---: | ---: | ---: |
  | 1 | 64 | 930.299 / 893.730 ms | **912.015 ms** | **14.250 ms** |
  | 8 | 8 | 180.237 / 167.948 ms | **174.093 ms** | **2.720 ms** |
  | 64 | 1 | 67.122 / 62.488 ms | **64.805 ms** | **1.013 ms** |

  Grouping edits reduces total work dramatically, but a 64-document group itself takes about **65 ms** before the newest updates become searchable. This does **not** warrant adding interactive wait time; the watcher already groups events over a maximum **80 ms** window.

- [PR #26](https://github.com/Lalalalendia/Akula/pull/26), **merged, test-only**: tests prove path deduplication and rescan preservation; [baseline workflow](https://github.com/Lalalalendia/Akula/actions/runs/38963487524) measured the former fixed delay: **80.087 ms** for one note, **80.137 ms** for a 64-note message and **80.109 ms** for eight queued 8-note messages. These measure queue collection, not search latency.
- [PR #28](https://github.com/Lalalalendia/Akula/pull/28), **merged, production behavior**: use **40 ms quiet window** after idle, retain hard **80 ms cap** from the first event, and temporarily use the full 80 ms window if a successful index update completed less than 80 ms earlier. No-op events do not activate cooldown. No change to commit durability or post-commit search visibility.
  - [Final integrated same-runner quiet-window workflow](https://github.com/Lalalalendia/Akula/actions/runs/38968027407): isolated note event-to-search median **93.129 → 52.635 ms**, saving **40.494 ms**. The corpus contains 512 notes; all post-commit marker checks passed.
  - Eight synthetic events at gaps of **5 / 15 / 30 / 45 / 60 ms** required **1 / 2 / 3 / 4 / 4 commits** with fixed 80 ms and **1 / 2 / 3 / 5 / 5** with the adaptive window. Thus the remaining trade-off is one extra commit in each tested slower-burst scenario.
  - All six final-head Actions succeeded: [Rust CI](https://github.com/Lalalalendia/Akula/actions/runs/38968027440), [paired performance](https://github.com/Lalalalendia/Akula/actions/runs/38968027397), [core benchmark](https://github.com/Lalalalendia/Akula/actions/runs/38968027486), [adaptive watcher](https://github.com/Lalalalendia/Akula/actions/runs/38968027407), [old watcher correctness](https://github.com/Lalalalendia/Akula/actions/runs/38968027448), [docs/installer](https://github.com/Lalalalendia/Akula/actions/runs/38968022543).
  - These are synthetic event streams on Linux CI, **not** real Windows/macOS desktop telemetry. Follow-up acceptance should include manual/automated OS-specific filesystem notification smoke tests and production-like edit bursts, without logging users' file contents or paths.

**Current strategy:** preserve immediate search visibility *after completed batches* and crash durability; improve naturally coalesced bursts rather than postponing single-note saves. Do not claim the 14× number for typical real-world editing without event-distribution evidence.

## Argon reader and UTF-16 follow-up (2026-10-17)

The second Argon audit covered text hyphenation, reader page layout, graph rasterization, UTF-16, rope/lines, and reader session visibility. **Do not merge proposed speedups without same-runner measurements and behavior-preserving tests.**

- [PR #38](https://github.com/Lalalalendia/Akula/pull/38), **merged** (`d289c85fe9858bbc1e02f99b00d03f398e67a94c`): reuse lowercase/trie/weight buffers in reader hyphenation and replace repeated break-to-layout + line-end searches with a sorted O(B + L) merge. Tests compare the prior algorithm against 1,500 mixed Russian/English/Unicode tokens, offsets and marked line ends; the existing reader page-layout and selection suite passed in full application CI. [Paired ABBA benchmark](https://github.com/Lalalalendia/Akula/actions/runs/38979407499), AMD EPYC 7763, Rust 1.99.1: 500 words **0.61564 → 0.55742 ms** per call, 5,000 words **6.16366 → 5.57612 ms** (**9.5% less isolated hyphenation time**). [Full workspace and sync-feature checks](https://github.com/Lalalalendia/Akula/actions/runs/38979407516) succeeded at the final PR head. This is **not** a measured whole-chapter layout or redraw speedup; the line-end mapping speedup was proven by complexity and correctness tests, not separately benchmarked.
- [PR #41](https://github.com/Lalalalendia/Akula/pull/41), **closed without merge**: Argon's single-pass UTF-8 byte classifier for `utf16_len` passed all UTF-16 correctness tests but was **31.7–35.0% slower** in eight short/long ASCII, Russian, emoji, and mixed-codepoint cases on the same runner. [Paired ABBA benchmark and raw logs](https://github.com/Lalalalendia/Akula/actions/runs/38979786140). The existing two-pass implementation remains in `main`. Avoid unverified claims about automatic SIMD vectorization.

Potential next experiments from the audit, **not yet accepted**: graph `Band::disc` solid-interior fast path, separable 3×3 dilation, chunked rope common-prefix matching, and no-allocation reader session visibility. Require visual pixel-level equality or bounded explained diffs for raster changes, unchanged text position/selection for reader changes, and same-runner paired timings. Avoid adding extra buffers to dilation unless the total memory and latency trade-off wins.

## Third optimization audit: inline parser, search hits, and surface labels (2026-10-17)

The third Argon audit suggested large multipliers for several hot functions. The multipliers were **hypotheses** and were never accepted as whole-app speedups. Only behavior-preserving changes with reproducible measurements entered `main`.

- [PR #85](https://github.com/Lalalalendia/Akula/pull/85), **merged** (`087fde0243f5910f06f09c15d7a9880d9bd5e523`): avoid `format!` calls when preparing 12 HTML tag pairs, skip HTML scans for paragraphs without `<`, and eliminate heap allocation for the common `==` / `%%` delimiters. Preserve Unicode byte offsets and alternative delimiters. [Same-runner source ABBA benchmark](https://github.com/Lalalalendia/Akula/actions/runs/39033186675): ~19.41× isolated ordinary-text marks, 14.23× long plain text, 2.62× delimiter marks, 1.36× tagged HTML and 1.18× mixed text. **No full-editor redraw gain has been measured.**
- [PR #89](https://github.com/Lalalalendia/Akula/pull/89), **merged** (`347ae6f43cec960610d8152796927f5db6ab1c50`): `stem_surface_labels` uses a lightweight analyzer on already-extracted alphanumeric words, retaining the same sequence of lowercase, folding, stemming and exclusion filters. Analyzer/schema version unchanged. [Same-binary ABBA](https://github.com/Lalalalendia/Akula/actions/runs/39034613648) measured **53.544 → 39.769 ms, 1.346× isolated speedup**. A separate index benchmark reported ~+2.2–2.4% elapsed time and SQLite +7.5%; these are **not** performance wins and no product-level or indexing speedup is claimed. The reusable ABBA CI now pins the exact PR base and uses separate Cargo targets for base/head (no repeated `cargo clean` on every sample).
- [PR #94](https://github.com/Lalalalendia/Akula/pull/94), **merged** (`928007ef0a00a27eee37e7ff19512c0191ddc82a`): generate global-search snippets through a single file handle. Early matches are one forward pass; distant matches rewind the same handle for heading-context prefix, and stale-index misses keep a bounded opening snippet rather than accumulating the entire file. Query-term filename matchers are reused. UTF-16 offsets, LF/CRLF normalization and headings have differential tests. [Same-process paired run](https://github.com/Lalalalendia/Akula/actions/runs/39034470033): **11.444 → 7.260 µs (1.576×)** for early matches; ~1.00× for distant hits and stale-index misses. **No whole-search latency improvement is claimed.**
- [PR #97](https://github.com/Lalalalendia/Akula/pull/97), **draft at this checkpoint**: reuse the already-computed raw BLAKE3 digest when LF-only content normalizes to identical bytes. CR and CRLF still require separate normalized hashes. Does **not** change `write_file_atomic_impl` or expected-hash/CAS ordering. Accept only after full CI and paired LF/CRLF measurements.

Deferred experiments require separate evidence and are **not merged as part of this audit**: AVX2 versus the current vello_cpu SSE4.2 cap (pixel-fidelity checks and real frame timing required); live-cover pixel-buffer reuse (never reinterpret `Vec<u32>` as `Vec<u8>` via `Vec::from_raw_parts` because allocator layout/alignment invariants differ); MCP title resolution through the index (stale/missing files must not authorize writes); and bounded concurrent Wikixiv HTTP (timeouts, rate limiting and cancellation). Moving a file's expected-hash check earlier is not a security fix and must undergo separate File Gate/TOCTOU review.

## Benchmark commands

From `aquilum-app`:

```sh
cargo test --workspace --locked
AQUILUM_BENCH_SIZES=1000 cargo test --release --locked -p aquilum-core benchmark_indexing -- --ignored --nocapture --test-threads=1
AQUILUM_BENCH_SIZES=1000 cargo test --release --locked -p aquilum-core benchmark_search_pipeline -- --ignored --nocapture --test-threads=1
AQUILUM_BENCH_SIZES=1000 cargo test --release --locked -p aquilum-core benchmark_link_pipeline -- --ignored --nocapture --test-threads=1
cargo test --release --locked -p aquilum-core benchmark_sqlite_metadata -- --ignored --nocapture --test-threads=1
cargo test --release --locked -p aquilum-core benchmark_save_breakdown -- --ignored --nocapture --test-threads=1
cargo test --release --locked -p aquilum-core benchmark_commit_batch_scaling -- --ignored --nocapture --test-threads=1
cargo test --release --locked -p aquilum-core benchmark_quiet_window -- --ignored --nocapture --test-threads=1
AQUILUM_BENCH_RICH=1 AQUILUM_BENCH_SIZES=1000 cargo test --release --locked -p aquilum-core benchmark_indexing -- --ignored --nocapture --test-threads=1
cargo test --release --locked -p aquilum-core benchmark_search_hit_window -- --ignored --nocapture --test-threads=1
cargo test --release --locked -p aquilum-core benchmark_surface_tokenizer -- --ignored --nocapture --test-threads=1
```

The SQLite-specific benchmark and both CI workflows are now available in `main`. Compare exact heads on the same CPU; runner-to-runner numbers alone are not sufficient.
