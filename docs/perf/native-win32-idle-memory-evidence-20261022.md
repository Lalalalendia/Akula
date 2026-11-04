# Akula M2 Windows 10-minute idle memory: diagnostic evidence (2026-10-22)

## Scope and exact provenance

- [Win32 dedicated Actions #39798875849](https://github.com/Lalalalendia/Akula/actions/runs/39798875849): **SUCCESS**; one native real Win32 app process with positive visible/uncloaked HWND, normal WM_CLOSE and clean exit.
- [Artifact #12435692652](https://github.com/Lalalalendia/Akula/actions/runs/39798875849/artifacts/12435692652): six evidence files (manifest, report, result JSON, one CSV, window witness, process log). ZIP digest `sha256:002c17c9c54a9ab29a071ba1b183a4629aa20d240b00c559bf57962dd5ee6474`.
- PR branch head before synchronization `e0f74a9735e22793d16b31493a3c7c29625199a5`; CI checkout was synthetic GitHub PR merge SHA **`5b5ae3f5c1da9eb862f92f6d28c690b2b7484c43`**, EXE SHA-256 `ca685d75b1e536d3a44b6bd4207890f910436719f4129ed2ff84474b0c53ca20`. Do **not** conflate the three identities.
- Windows hosted CI with `cargo build --release --locked -p aquilum-native --features installed`, **unpackaged**, isolated empty synthetic vault/data-dir, 1000×650 viewport. No app, File Gate, sync, security policy, Rust code or instrumented memory-probe changes.
- First 60 seconds of process lifetime is **settle**, followed by 600 seconds idle, nominal 1 Hz PSAPI samples: **660 total CSV rows, 600 idle rows**. PID/creation-time consistency, no forced kill, stable visible window witnessed. The first HWND sample at 1.0s is not proof of first usable interaction.

## Raw facts (one process, not a production median)

| Metric | Value |
|---|---:|
| Private Bytes: first 30 seconds of idle median | **14.391 MiB** |
| Private Bytes: last 30 seconds of idle median | **14.387 MiB** |
| Private Bytes median change over 10-minute idle | **−0.004 MiB** |
| Private Bytes full idle sampled min/max | **14.297 / 14.523 MiB** |
| Descriptive linear slope across 600-second idle | **−0.061 MiB/h** |
| Working Set: first/last 30 seconds medians | **52.941 / 16.309 MiB** |
| OS process Peak Working Set high water | **53.094 MiB** |
| CPU time between first and last idle samples | **63 ms** |
| Process handles first/last 30 seconds median | **380 / 372** |

Private Bytes and Working Set are **not interchangeable**. Working Set decreased dramatically (52.94 to 16.31 MiB) with nearly unchanged Private Bytes, consistent with page trimming/reclamation by Windows rather than release of 36 MiB of private allocations; the actual mechanism was not measured and needs additional counters if attribution matters. A single run with flat Private Bytes is **not** proof that arbitrary documents, opened graph, galleries, long-running sync or index do not leak. There is no user-facing first-interactive latency measurement here.

## Implications and follow-ups

1. **Accept M2 as diagnostic-only idle evidence:** no obvious private committed memory accumulation or CPU spin for *one empty vault over 600 seconds*. It is NOT the installed NSIS product baseline and cannot establish product-wide memory budgets.
2. Do not rewrite FileTree or Vello for this result. B2/B3 [Actions #39780825738](https://github.com/Lalalalendia/Akula/actions/runs/39780825738) shows an additional ~23.98 MiB Private Bytes for 50k expanded-vs-collapsed rows at warm end, but is also single-cell diagnostic data. Next high-leverage steps are 5 installed NSIS repetitions, UI Automation first usable frame and M3/M4 realistic 100k-line editor and 20 tabs, rather than endless empty process tests.
3. **Security:** historical required PR check failure was caused by pre-remediation RustSec watchdog; main remediation [PR #595](https://github.com/Lalalalendia/Akula/pull/595) is now merged at `0bef54b73d00a68b5996bfc34ea10f22eac2f330` and a fresh [RustSec watchdog #39799386558](https://github.com/Lalalalendia/Akula/actions/runs/39799386558) PASSED. Branch synchronization and new required PR checks are still needed before any merge; never waive/fabricate a green required check.
4. Any new CI on synchronized main will compile the new Cargo.lock, thus produce a **different EXE SHA** and independent new evidence. Keep old evidence with old source identity; compare only where the environment is controlled.

Canonical protocol: [AKULA-PERF-MEMORY-01](https://app.notion.com/p/40032a84beec8104841cd8da8f78e33b).
