# Vello: exact-coverage batching of adjacent damage regions (experiment)

## Why this experiment exists

The [0/2/4 worker study](https://github.com/Lalalalendia/Akula/pull/162), [persistent dual-dispatch trial](https://github.com/Lalalalendia/Akula/pull/166), and [2/4/8/16 strip falsifier](https://github.com/Lalalalendia/Akula/pull/173) show that *the number and shape of damage rectangles* can matter more than the total dirty pixel area.

Measured in #173 on the same Linux release runner (text 1x, full refresh every third frame):

| Rasterization | Fixed serial | Aggregate-area router |
|---|---:|---:|
| 2 touching strips | 39.35 ms | 35.32 ms |
| 16 touching strips | 55.06 ms | 75.53 ms |

This does **not** imply that joining unrelated rectangles by bounding box is safe. Expanding the dirty union renders previously untouched pixels, can change alpha blending, and may reintroduce clipped-glyph alignment problems. The guard96 falsifier [#179](https://github.com/Lalalalendia/Akula/pull/179) has a separate >1/255 mismatch that must remain visible, not waived.

## Existing upstream ideas, not reinvented

- Vello CPU already batches path operations by estimated path complexity and `COST_THRESHOLD` (see `native/vendor/vello_cpu/src/dispatch/multi_threaded/cost.rs`).
- oneTBB's *agglomeration* and partitioner guidance suggests grouping tasks to amortize scheduling cost, not naively multiplying threads.
- Vello's `MultiThreadedDispatcher::reset` uses `spawn_broadcast` plus a barrier; each separate damaged rectangle also repeats `Painter::begin`, scene replay and flush. Rendering many thin rectangles can make this cost dominant.

## What is actually implemented

Test-only `coalesce_adjacent` merges two damage rectangles **only when their entire edges touch and their other coordinate and dimension match**. It refuses to process **any** list containing overlapping or empty rectangles. Combining horizontal/vertical neighbours is iterative and coverage-preserving: the input and output have exactly the same written pixel set and exactly the same summed area. It cannot introduce a bounding-box gap. Normal Akula binaries do not include the coalescer or its callsite.

The A/B uses the same editor fixture, transforms, renderer cache and visual layers in four modes: fixed 0/2 workers with original fragmented regions versus fixed 0/2 workers with exact-footprint coalescing. Each full redraw is split into 2/4/8/16 touching bands before the test-only coalescer runs. The benchmark requires:

1. Original and coalesced framebuffer **bitwise equality on every frame**.
2. Max **1/255 per-channel** discrepancy from a fresh independent full-frame reference.
3. Exactly the same scheduled pixel count, **N regions before, 1 region after** full redraw.
4. No merging of overlaps or gaps in direct tests.
5. Three independent release processes, full 576-row report, raw logs preserved.

## Per-frame CPU instrumentation

The test-only `FrameStages` now separates:

- `reset`: `painter.begin` including `RenderContext::reset` and worker synchronization; the barrier is **not** separately measured.
- `replay`: Masonry scene replay and preparation/encoding for the current region.
- `flush`: `RenderContext::flush`, including synchronization of pending work.
- `raster`: `RenderContext::render_with`, including the raster-stage dispatch.
- `encode`: existing combined time for reset/replay/flush; `total`: entire `draw_frame` invocation.

Stage measurements are inclusive wall times for isolated calls. They **must not** be interpreted as proof that all CPU work in `reset` comes from a barrier or `spawn_broadcast`. A separate Vello-internal probe would be needed for that attribution.

## Go/no-go

- Failing pixel parity, unexpected increase in dirty pixels, or inconsistent timing records: **stop**. No fallback that hides an artifact.
- If full-edge coalescing consistently reduces total time and especially reset/replay/flush time, evaluate a more general batching API that preserves a list of exact dirty rectangles without joining non-neighbours. This would likely require an upstream/vendor renderer interface, and must preserve z-order/transparency and clip semantics.
- If results are noisy or consistently slower, leave production routing unchanged and investigate scene replay caching and dispatcher granularity independently.
- Before production: Windows/macOS fidelity; memory/RSS and source-cache behavior; sparse/overlapping damage; glyph and alpha edge regressions; native UI tail-latency.

Reference sources:
- [Vello CPU cost model](https://github.com/linebender/vello/blob/main/vello_cpu/src/dispatch/multi_threaded/cost.rs)
- [Vello CPU dispatcher](https://github.com/linebender/vello/blob/main/vello_cpu/src/dispatch/multi_threaded.rs)
- [oneTBB agglomeration](https://www.intel.com/content/www/us/en/docs/onetbb/developer-guide-api-reference/2022-1/agglomeration.html)
