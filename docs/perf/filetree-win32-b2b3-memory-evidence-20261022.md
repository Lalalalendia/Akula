# AKULA PERF B2/B3 — Windows FileTree 1k/10k/50k diagnostic evidence (2026-10-22)

## Provenance and result

- [Dedicated Windows CI #39780825738](https://github.com/Lalalalendia/Akula/actions/runs/39780825738): **SUCCESS**, 18 of 18 isolated native Win32 GUI process runs; [artifact #12429881033](https://github.com/Lalalalendia/Akula/actions/runs/39780825738/artifacts/12429881033), artifact SHA-256 `7524e5ea73d228be52e8dfd6e535b81a351ab576cc84d52d70687e0728d5df49`.
- PR branch head `588c46be66eceb32d4f3ccb7f7edf07bb5642aea`; checkout/source recorded in experiment is GitHub synthetic PR merge `98380432d04ddf6b2b15f24b0fc587afdda775cd`. Binary SHA-256: `59d8f1db6618ea5a683617e9454f02f5bf28920b6e5e56780dcb40ae19039032`. These are different identities and must not be conflated.
- Windows 2025 runner, primary desktop 1024 × 768, application viewport 1000 × 650. Regular uninstalled Windows Release binary built with Cargo feature `installed`; **not physical NSIS installation**. All files are synthetic.
- Each of three sizes (1000/10000/50000 Markdown files) was measured as flat, nested-collapsed, nested-expanded, separately fresh/reused profile: 18 independent processes. Each cell has **one run only** (not an ABBA comparison or statistical baseline). 40/80/160 s observation and nominal 200 ms PID/creation-time-bound PSAPI sample.
- Raw ZIP independently inspected: 57 files (environment.json, results.json, summary.md, 18 × samples.csv/witness.json/process.log); 18/18 graceful exits; no forced kills; positive uncloaked visible Win32 window checks. CPU, Working Set, sampled Private Bytes, process peak Working Set and SQLite metadata count captured. Source sentinels SHA-256 and note counts checked. There are no user files.

## End-of-window memory in MiB

The important distinction: **Private Bytes is private committed virtual memory**, not Working Set or GPU memory. Peak Private Bytes below is *highest sampled value*, while Peak WS is an OS high-water counter.

| Files | Tree | Cold final Private | Cold sampled peak Private | Cold peak WS | Reused-profile final Private | Reused-profile peak WS |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1,000 | flat | 18.96 | 22.28 | 60.96 | 15.80 | 53.37 |
| 1,000 | nested collapsed | 15.43 | 44.71 | 68.96 | 13.22 | 51.01 |
| 1,000 | nested expanded | 17.78 | 46.55 | 71.30 | 16.42 | 53.72 |
| 10,000 | flat | 24.52 | 64.77 | 92.56 | 22.95 | 61.74 |
| 10,000 | nested collapsed | 19.80 | 62.51 | 88.25 | 16.52 | 56.83 |
| 10,000 | nested expanded | 25.04 | 66.34 | 94.20 | 23.32 | 62.13 |
| 50,000 | flat | 42.17 | 98.97 | 124.06 | 42.05 | 81.09 |
| 50,000 | nested collapsed | 22.25 | 81.33 | 108.26 | 19.80 | 64.06 |
| 50,000 | nested expanded | 45.04 | 108.12 | 127.14 | 43.78 | 83.95 |

Reused-profile expanded-vs-collapsed Private Bytes differences: **+3.20 MiB** (1k), **+6.80 MiB** (10k), **+23.98 MiB** (50k). With flat layout all files are root entries and therefore materialized without expanding subfolders. These results are compatible with retained file-tree model rows/paths; **the experiment cannot isolate the exact memory owner**.

At 50k cold-run observed Private peaks span **81.33–108.12 MiB**, but final Private returns to **22.25–45.04 MiB**. `t≈30 s` values (flat 42.37, collapsed 22.45, expanded 45.27 MiB) remain within approximately 0.2–0.3 MiB of `t≈160 s` (42.17, 22.25, 45.04 MiB). No ongoing increase in this 130-second tail, **not a leak-freedom proof beyond 160 s**.

## Startup and index timing caveats

- First **uncloaked visible HWND proxy** on 50k: cold flat 0.391 s, collapsed 0.203 s, expanded 0.406 s. Warm flat 0.407 s, collapsed 0.203 s, expanded 0.407 s. 200 ms polling quantizes these values; not first interactive frame and not response to user input.
- First observed `documents.sqlite3` count=N on 50k cold: 8.031 s flat, 10.031 s collapsed, 8.047 s expanded. **This is SQLite metadata count only**, not proof that Tantivy finished its actual scan or queries, and the observation resolution is 2 seconds.
- All warm profiles report `metadata_complete_s=0.0` because SQLite records already exist. Calling this `index ready at 0.0 s` would be **wrong**. Need a real worker/progress and query-usability witness for a defensible index-ready timestamp.
- Test stderr has repeated `[aquilum:history] migration refused .../.trash: history workspace root is missing` diagnostic message. No file-integrity failure was detected in the synthetic sentinels, but this message was **not** investigated as a correctness pass of History/Trash. Do not silently treat it as a memory leak.

## Engineering decision

The relative increase for *50k materialized rows* is real enough to prioritize a focused confirmation **but not enough to justify changing production architecture**: ~24 MiB is not itself a proven end-user bottleneck. Prioritize ten-minute process idle with 60-second settle (M2); then a paired/counterbalanced 50k collapsed-vs-expanded probe and first-interactive UIA gate if the product scenario is worth optimization. Defer `FileTree` row restructuring, graph cache removal, and scanner-cancellation changes until correctness, latency, and memory budgets are measured together.

## CI security HOLD

[Application checks #39780825912](https://github.com/Lalalalendia/Akula/actions/runs/39780825912) = **FAILURE**, while this optional diagnostic workflow = **SUCCESS**. The mandatory check fails because [main-branch dependency watchdog #39771198572](https://github.com/Lalalalendia/Akula/actions/runs/39771198572) found **13 RustSec vulnerabilities** in the baseline lockfile. Do not suppress, bypass or mark the required check green; [security Draft PR #595](https://github.com/Lalalalendia/Akula/pull/595) is the separate remediation path and remains blocked on additional E2E acceptance. **Keep B2/B3 PR #605 Draft/unmerged.**

Canonical design and measurement protocol: [AKULA-PERF-MEMORY-01](https://app.notion.com/p/40032a84beec8104841cd8da8f78e33b), [AKULA-ARCH-MEM-02](https://app.notion.com/p/40032a84beec817a93d3ce1d750fd8cd).
