# Parley layout reuse: release decision (2026-10-18)

**Decision: HOLD. Do not ship the clean-tab Parley layout cache from PR #117.**

This record covers a specific proposal: transfer pre-shaped, plain, non-interactive
Parley paragraph layouts between clean Markdown editor tabs. It does **not**
disable the production BLAKE3-verified clean `Rope` cache or the 1536-line
initial editor window.

## Evidence and diagnosis

- [PR #116](https://github.com/Lalalalendia/Akula/pull/116),
  merged **test-only**: a 69 KiB isolated `Document` ABBA was **1.352x
  faster**. It did not measure user-visible tab switching.
- [PR #118](https://github.com/Lalalalendia/Akula/pull/118),
  closed unmerged after [UI diagnostic
  run](https://github.com/Lalalalendia/Akula/actions/runs/39047004104):
  630 actual restored paragraphs across 64 / 512 / 4096 KiB test
  cases; p95 results were mixed and 64 KiB regressed.
- Initial zero hit counts were **not** a file-cache miss. The live
  editor initially measured a 361 px wrap width versus 345 px in the
  retired widget (16 px scrollbar transition). Strict width equality
  correctly rejected previous-wrap geometry. The trial now permits
  eligible shaped glyph runs to be rebroken at the **new actual width**
  through the normal `Document::measure` machinery. It must never
  transfer stale line heights or bypass normal line breaking.
- [PR #122](https://github.com/Lalalalendia/Akula/pull/122),
  merged **test-only**, measured real widget switches, 252 restores,
  eight exact pixel-frame comparisons and BLAKE3 rejection of
  same-length external edits.
- [PR #130](https://github.com/Lalalalendia/Akula/pull/130)
  rejects layout capture with IME preedit, selection or any secondary caret.
- [PR #134](https://github.com/Lalalalendia/Akula/pull/134)
  matched complete pixel buffers at physical scales 1.0x and 1.75x
  and logical widths 1000 and 860, **under the test-only path**.

Those tests support *correctness of the experiment*, not a product win
or validation of the divergent product implementation.

## Repeated end-to-end release-profile result

[PR #135 / Actions run 39051772963](https://github.com/Lalalalendia/Akula/actions/runs/39051772963)
completed three same-runner release-profile ABBA passes, 24 measured
switches per arm, per note size, per pass. Each candidate pass
actually restored 240 eligible paragraphs; the metric includes
full native tab switching and first redraw.

| Note size | Baseline p95 median | Candidate p95 median | Candidate / baseline p95 |
| --- | ---: | ---: | ---: |
| 64 KiB | 1.78 ms | 2.36 ms | 1.33x (regression) |
| 512 KiB | 7.58 ms | 6.56 ms | 0.87x |
| 4096 KiB | 9.31 ms | 9.44 ms | 1.01x |

Independent 4096 KiB paired p95 ratios: **1.161x, 0.764x, 0.920x**.
The workflow recorded `PARLEY_RELEASE_DECISION=HOLD_PRODUCTION_CACHE`.
Small-note regression is repeatable across all three trials; 4 MiB
benefit is not stable. Do not extrapolate the microbenchmark's 1.352x
number to Akula.

## Disposition and constraints

- [PR #117](https://github.com/Lalalalendia/Akula/pull/117)
  **closed without merge**, as it diverged from main's independent test
  hooks and could not justify release cost/risk.
- [PR #135](https://github.com/Lalalalendia/Akula/pull/135)
  **closed without merge**, diagnostic evidence preserved in CI artifacts.
- Production still uses safe, hash-verified Rope reuse (PR #96), a
  1536-line editor window (PR #104) and ordinary fresh Parley layout.
- Do not ship a global layout cache, invalidate less aggressively,
  or change raw-file BLAKE3 verification merely to increase cache hits.

If the hypothesis is revisited, start from **current main** and use a
small, separately reversible experiment. Require all of the following:
actual nonzero eligible reuse, realistic Markdown/Unicode corpora,
repeated paired **release** whole-switch + first-frame measurements
with small/common notes included, memory bounds, pixel equivalence
across DPI/theme/font/width, dirty edits/external changes, IME,
multi-cursor, scroll jumps, file rename/close and workspace switch,
plus the final-head full Rust/sync-engine and frame-fidelity gates.
Only a **consistent material net benefit without important regressions**
could warrant production integration.

Do not treat a passing diagnostic workflow as a release authorization.
