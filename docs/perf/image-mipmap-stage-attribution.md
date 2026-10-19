# IMG-PERF-03 — Per-frame mipmap / encoder / Vello cost attribution

Status: diagnostic-only draft. Parent: [IMG-PERF-02 PR #381](https://github.com/Lalalalendia/Akula/pull/381), merged as 0454381. Prior Windows replay-only CPU p95/p99 is established, but cannot attribute the sampling and image preparation stages.

## Exact scope

Two opt-in environment variables are used by the developer-only Windows replay benchmark:
- `AKULA_EXPERIMENTAL_FRAME_SAMPLES=1` (IMG-PERF-02): replay-only CPU work samples, plus historical full-process frame samples without stage attribution.
- `AKULA_EXPERIMENTAL_FRAME_STAGES=1` (this PR): `metrics.json.frameStages` **includes frame 1 and startup**, while `replay.json.stageSamples` includes only **presented frames inside the scroll replay**. A row is valid only when both arrays have complete frame counts and finite, nonnegative metrics.

Fields per stage row (durations in milliseconds):
- `sceneMs`: Masonry/app rewrite + redraw before renderer.
- `rendererDrawMs`: aggregate native renderer.draw, including frame changes, commands, Vello and side effects; NOT pure rasterization.
- `encodeMs`: accumulated per-damage-region painter.begin, scene replay, and painter.finish; **includes mipmap allocation and reduction**.
- `mipBuildMs`: independent subset of encodeMs from cumulative mip-build timers.
- `velloMs`: wall time around `RenderContext::render_with`; not GPU scanout or per-image sampler alone.
- `scrollShiftMs`: CPU wall time spent in row-shift work for the frame.
- `outputPresentMs`: native app output present invocation + adjacent post-present UI bookkeeping; not a hardware present timestamp.
- `rasterPixels`: scheduled damaged pixels (areas can overlap; this is **not** unique pixel coverage).
- `shiftedPixels`: scroll-copy workload area; it is not per-image paint area.
- `rasterRects`: scheduled damaged rectangle count.
- `mipGenerated`: number of generated levels during the frame.
- `mipHits`: final-level cache hits during the frame.

Stage clocks are sampled for **both baseline and candidate**, using identical opt-in instrumentation. Normal developer and installed Akula builds with the flags absent retain the existing paths. This does not change mip thresholds, cache policy, pixel blending, Vello dispatch or installed features.

## Expected findings and falsification

1. **Cold-frame attribution.** Verify that the first observed candidate levels are generated in a specific frame in `frameStages`, with a corresponding `mipBuildMs` spike within `encodeMs`. Baseline must have zero mip builds across all startup and replay frames. The earliest decode phase, before `Renderer::draw`, is *not* measured by mipBuildMs.
2. **Warm replay comparison.** Compare distributions of `replay.workSamplesMs` and stage `encodeMs`/`velloMs`, paired for the three trials. A candidate may win on Vello and lose on encode, or vice versa.
3. **Partial-damage effects.** Compare nonzero shiftedPixels vs rasterPixels. A shifted source row does not cause Vello to filter its image again. Neither metric is a direct image coverage counter.
4. **Remaining experiments.** Add per-image actual painted-area counters (image brush intersection with final damage rect), multiple distinct source images and forced cache eviction. Compare scalar 2x2 box, exact-byte packed reduction, SIMD/resampling and early decode downscale only after these stage costs are separated.

## Acceptance checks

- [ ] All required Rust/native Windows/Windows beta and application CI green on exact SHA.
- [ ] Python benchmark syntax valid and complete 12-process real Win32 GUI matrix.
- [ ] Every whole-process stage row accounts for all frames including first and every replay stage row matches a presented replay frame.
- [ ] Baseline has zero generated mip levels; candidate builds >=2; input hashes and window witnesses remain identical.
- [ ] Installed rendering is unchanged; do not ship mipmap or conflate CPU work with hardware frame pacing.

The diagnostics are a falsifier, not a performance optimization or release authorization.
