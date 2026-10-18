# Native Vello: natural damage cost attribution

## Why the batching-only hypothesis is insufficient

The successful existing [Native Damage Tracker stage profile](https://github.com/Lalalalendia/Akula/actions/runs/39076704984) measured 48 editor frames, including 41 detected scroll shifts, with a maximum of **one final raster rectangle** in both reference scenes:

| Natural Akula scene | Raster rectangles (max) | Raster area p50 |
|---|---:|---:|
| 1000×600 at 1× | 1 | 2.540% |
| 1750×1051 at 1.75× | 1 | 1.397% |

Therefore, merging adjacent rectangles cannot accelerate *these* normal scroll frames. The synthetic 2/4/8/16-strip studies [#173](https://github.com/Lalalalendia/Akula/pull/173) and [#187](https://github.com/Lalalalendia/Akula/pull/187) are useful only for stress-testing fragmented redraws, and should not determine Akula's shipping worker policy.

## Measurement plan

This experiment instruments the native renderer under `#[cfg(test)]` only, without forced full frames, damage guards, artificial region splits, caching changes, or production thread routing.

- `record` and `detect`: existing Damage Tracker stage measurements.
- `reset`: local `Painter::begin` time, including `RenderContext::reset` and parallel-dispatcher wake-up/barrier.
- `replay`: complete Masonry recorded scene replay and Vello command encoding for the current dirty region.
- `flush`: Vello `RenderContext::flush` time (pending commands complete).
- `raster`: Vello `render_with` time, including glyph cache preparation, rasterization and post-render maintenance.

The test uses 0, 2 and 4 persistent worker contexts on **identical native Akula frames** and rotates execution order to reduce thermal/cache bias. On every frame it requires bitwise equality across worker counts with identical damage geometry. Four independent full-reference checkpoints per scene must differ by at most 1/255 per 8-bit channel.

It reports the number of real scroll frames with more than one dirty rectangle, and the number having disjoint rectangles sharing a complete edge (the only cases eligible for lossless adjacent coalescing).

Three independent release processes are required on the same GitHub Actions runner; the Python aggregator rejects missing records, mode-dependent damage geometry, implausible region counts or insufficient natural scroll activity.

## Decision gates

- If natural frames are nearly always one region, stop optimizing rectangular coalescing for ordinary scroll. Retain it as a narrow special-case study only if other real workloads justify it.
- If `replay` is large for one dirty region, study reusing prepared Parley/Vello scene geometry and/or reducing `frame.paint_into` scope while preserving clipping.
- If `reset` or `flush` dominate for multi-thread mode, perform a separate Vello-internal broadcast/barrier probe before adjusting any worker policy.
- If `raster` dominates, optimize culling, glyph preparation or the rasterizer. Compare the 0/2/4 crossover against real damage area, not synthetic strip count.
- Maintain production pixel fidelity and fallback, and require Windows/macOS/memory gates before shipping.

This is instrumentation, not a performance patch. The known diagnostic guard96 >1/255 mismatch from [#179](https://github.com/Lalalalendia/Akula/pull/179) remains a separate correctness issue.
