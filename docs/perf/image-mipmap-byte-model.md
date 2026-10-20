# IMG-PERF-02: image mipmap byte cost and replay measurement repair

Status: experimental, diagnostic only. No installed-renderer change.

## Evidence and measurement problem

Windows evidence: [PR #373](https://github.com/Lalalalendia/Akula/pull/373), exact commit 2a19019, [successful Windows workflow](https://github.com/Lalalalendia/Akula/actions/runs/39282960136), artifact 12202213629.

- Source: native/src/metrics.rs records **all frames after the first**; replay.rs records **only nonempty frames during the scroll replay**. The old benchmark labeled startup-inclusive frameP95Ms as scrolling performance. Roughly 260-264 total frame samples vs 248-249 scroll replay frames per run. That contaminates the upper five percent of samples.
- Source: host/frame.rs starts its raster timer immediately before the entire renderer.draw() call. This includes command encoding, mip generation and rasterization; rasterP95Ms is **not** isolated Vello texture sampling.
- Source: replay.rs compares frame intervals to a 120Hz constant; on a ~60Hz Windows hosted desktop, its missed counter is not a 60Hz dropped-frame metric.
- Fix in this PR: write optional **replay-only CPU workSamplesMs** (already collected in Replay.work) into diagnostic replay JSON; Windows A/B must compute p95/p99 from those samples instead of startup-inclusive totals. Baseline JSON remains unchanged when not in explicit developer sample mode.

## Byte-level cost model, exact pinned 1280 x 884 source

| Image | Dimensions | Output pixels | RGBA bytes |
| --- | --- | ---: | ---: |
| Original | 1280 x 884 | 1,131,520 | 4,526,080 |
| Mip L1 | 640 x 442 | 282,880 | 1,131,520 |
| Mip L2 | 320 x 221 | 70,720 | 282,880 |
| Extra L1+L2 | | 353,600 | **1,414,400 (1.349 MiB)** |

The code in render/paint/mipmap.rs builds successive premultiplied-RGBA8 2x2 box averages. It reads 4 source RGBA pixels (16 bytes) per output RGBA pixel (4 bytes). Useful logical data traffic for the two levels is **5,657,600 bytes read + 1,414,400 bytes written**, about 7.07 MB, before accounting for cache lines, instruction overhead and allocation. Two levels were generated in each Windows candidate run, with 0 cache evictions. Generation cost from diagnostic counters: 4.33-7.15 ms.

At a displayed width of 200px, scale 200/1280=0.15625; the current level selector chooses L2, after which Vello samples a 320px source at scale 0.625. Precomputed image pixels are not free: total generated output area is 353,600 pixels versus a final 200 x ~138 image.

## What the old benchmark actually established

The startup-inclusive median p95 on exact-head Windows:
- compact 768 x 522: 1.46 ms baseline, 1.50 ms mip+Medium+512;
- large 983 x 675: 1.71 ms baseline, 1.94 ms mip+Medium+512.

Excluding 16 earliest total-process samples *as an exploratory proxy, not exact replay alignment*, median p95 changes to approximately 0.95 vs 1.06 ms compact and 1.14 vs 1.05 ms large. The corrected exact replay-only p95/p99 requires a new workflow on the new HEAD, not interpolation from the old archive.

Worst CPU frame: 5.332 vs 13.442 ms compact; 5.885 vs 9.550 ms large. The expensive candidate frames are within the first two sampled frames and mip generation occurs once. Separate timestamps are still required to prove direct attribution. With a single repeated source, candidate reports approximately 100-116 mip hits, no evictions. This cannot validate multi-image LRU behaviour.

## Hypotheses to falsify

1. **Measurement mixture**: old p95 reflects startup+scroll rather than scroll. This PR fixes it.
2. **Synchronous generation**: pixel iteration and two new buffers block renderer.draw() in cold frames. Measure alloc/build/encode/render independently.
3. **Low damaged image area**: Vello samples *visible rasterized pixels*, not all original source pixels, while mip generation processes the whole source. The benchmark includes substantial non-image text.
4. **Different implementations**: original Vello image painters are SIMD-capable; the Akula 2x2 average uses scalar Rust loops. Inspect assembly, not merely source, before claiming it never vectorizes.
5. **Cache economics**: 512-frame long-lived cache and 32MiB budget appear in developer mode, but one source with zero evictions is not a realistic working set.
6. **Image fidelity**: upstream Vello #1289 documents insufficient source coverage for >2x minification. Evaluate aliasing, edges, cropped areas, alpha, gamma, HiDPI and transforms even where FPS does not improve.

## Next test matrix

- Gather replay-only p95/p99/mean/max and separately preserve process-startup p95, using paired trials and confidence intervals.
- Record exact per-frame times for mip allocation, L1/L2 build, warm lookup, eviction, encoding, Vello rasterization, and output present. Tag pre-roll vs replay.
- Benchmark half() scalar, row-indexed and SIMD candidates with identical output bytes and reference-image comparisons; include opaque/transparent and odd dimensions, 1280px and 4K inputs.
- Exercise 1, 10, 100 distinct images and 200/400/800px render widths, different zoom/density, forced LRU eviction, cold and warm passes.
- Compare baseline+High, baseline+Medium, mip+High, mip+Medium, mip+Medium+LRU and predecoded display-size bitmap.
- Consider async mip generation or earlier decode-to-display-size only *after* measuring. Retain source for sharp zoom and preserve fidelity gates.

**Break-even example:** 5 ms generation with 100 paint uses requires >0.05 ms saved per image paint merely to pay for build; this excludes extra cache costs and cold-frame latency. A paint use is not necessarily a distinct rendered frame.

Do not activate mipmaps in installed Akula or change renderer defaults based on this diagnostic PR.
