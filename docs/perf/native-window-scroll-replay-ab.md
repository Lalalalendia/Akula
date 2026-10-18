# Akula actual-window scroll replay A/B

## Scope and motivation

Merged PR #265 improved Vello raster time, and PR #272 measured an actual
pointer-event -> Masonry scene -> **CPU framebuffer** improvement on Linux.
Neither includes the real Winit window and softbuffer present/copy path.

This diagnostic deliberately **reuses** the existing native executable,
`--replay-scroll`, `--metrics`, Winit/Masonry host, and softbuffer output.
It does not create a new benchmark renderer or change the shipping 350,000-
pixel region limit, worker-count defaults, glyph rendering or visual fidelity.

The replay instrumentation corrects two pre-existing issues: a no-op redraw
with zero changed regions no longer counts as a submitted frame; and a long
interval above 30 ms is serialized as a valid JSON [index, milliseconds]
pair, rather than a Rust tuple that makes the entire report invalid. A real
nonempty-damage frame is recorded after the existing surface present call.
The second case is covered by a deterministic Rust regression test.

## Automated Linux Xvfb A/B

The workflow builds the normal uninstalled **release** native binary once.
Under one Xvfb 1920x1200 display and Openbox instance, a single Python
driver launches separate Akula GUI processes in this order:

1. Trial 1: fixed2, adaptive2.
2. Trial 2: adaptive2, fixed2.
3. Trial 3: fixed2, adaptive2.

Each mode runs two self-contained fixture notes: a long plain Markdown
note at 1000x600 and a static patterned cover note at 1750x1051. The suffix
"large" refers to physical window dimensions, **not** a 1.75x DPI scale.
Every process uses identical note bytes, a distinct data directory and
the same explicit `--threads 2`. The production constructor is controlled
by `AKULA_VELLO_SINGLE_RECT=0` (fixed) or `=1` (adaptive); the legacy
variable is deliberately removed.

The driver requires the replay JSON and metrics JSON to exist and contain
nonempty frame counts, four replay quarters, positive interval/work data,
and a matching thread count. Invalid/missing GUI evidence fails the test.
It reports per-mode medians of submitted-frame FPS, submission interval p95,
average CPU frame work, host frame/raster p95, and archives all individual
raw logs and JSON. **No minimum performance gain is enforced in CI.**

## Interpretation and limits

- Xvfb is a **virtual** window server. Results measure native window and
  softbuffer present-path work, not physical monitor scanout, GPU timing,
  human-perceived FPS or real mouse-to-photon latency.
- The existing replay counter for "missed" intervals assumes **120 Hz**;
  this is not a measured display refresh rate on the Xvfb runner. Do not
  interpret this number as physical dropped frames.
- FPS may be capped by the replay/vsync pacing even if CPU work improves.
  Use workAvgMs and host raster/frame p95 alongside FPS and interval p95;
  positive CPU gain need not change the submission cadence.
- The UI run is not a side-by-side pixel-fidelity gate: PR #260/#265/#272
  separately preserve strict rendering parity. The two run-specific UI
  sessions can have different metadata pixels or font caches.
- A local desktop can run the same driver against its normal native binary,
  inside a real graphical session. Only the same-machine, same-display A/B
  can support a user-facing smoothness comparison; even that does not directly
  measure scanout or photons.

## Running locally

With Python 3, a native release binary, and a working GUI session:

```text
python3 scripts/benchmarks/native_scroll_window_ab.py --binary PATH_TO_NATIVE_BINARY --out PATH_TO_RESULTS
```

The driver creates only synthetic note files and private temporary
application data beneath the selected results folder. Nothing is uploaded
unless the caller explicitly chooses to archive the results.
