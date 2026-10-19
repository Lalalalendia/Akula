# IMG-PERF-07 — Isolate Vello sampler quality from mipmap LOD

Status: **developer-only diagnostic**, one pinned photo in 24 *real* native Win32 application processes. The installed feature profile cannot read the experimental flags and remains unchanged.

## Why

The first 12-run A/B conflated (1) building/caching a mipmap with (2) switching Vello's High/bicubic image interpolation to an optimized Medium/bilinear path. Warm scrolling reuses existing BGRA rows, so the resampled image occupies only a small proportion of actually damaged screen pixels. Prior #386 per-frame trace found only ~0.04–0.05ms Vello `render_with` for a typical 0.7–0.75ms CPU replay frame. Later #392 and #401 showed material cold-build and cache retention improvements, but they did not independently quantify a sampler-filter change.

## 2×2 experimental design (same 1280×884 pinned PNG and image geometry)

| Cell | Mipmap | Vello sampler |
| --- | --- | --- |
| baseline | OFF | Original High |
| filter_medium | OFF | Medium |
| mip_high | ON, existing eager L1→L2 | High |
| mip_medium | ON, existing eager L1→L2 | Medium |

Only the `filter_medium` mode is new. The factorial forces **High or Medium on each of the four experimental modes** because the application's native brush may already default to Medium. It applies `ImageQuality::Medium` to image brushes without allocating mipmaps or modifying source identity, pixel dimensions, alpha, transform, crop, or cache. The `AKULA_EXPERIMENTAL_IMAGE_FILTER_FORCE=high|medium` flag works only in **uninstalled developer builds**; distinct one-time `AKULA_FILTER_FORCED_HIGH_APPLIED` / `AKULA_FILTER_FORCED_MEDIUM_APPLIED` witnesses prove the actual image brush path used in every cell. Mip candidates must show the existing `AKULA_MIP_APPLIED` witness; baseline and Medium-only must show neither.

A production quality decision CANNOT be made on CPU timings alone. Medium without mipmap may alias strongly on 6× minification; mipmap coverage can improve fidelity at higher cost. Compare pixel fidelity in separate source-pattern and image-level tests.

## Exact-head CI experiment

- One pinned developer release binary SHA and image/note hashes across modes; input hashes must be identical.
- Three paired trials per cell at each of two actual desktop-supported window dimensions, 24 process invocations total. Reverse order on alternate trials.
- Require visible PID-owned uncloaked Win32 window, nonzero peak working set, frames and per-frame CPU/stage metrics, positive/negative filter and mip witness.
- Report replay-only CPU p95/p99 and per-stage encode/Vello mean, cold mip build, peak RSS. Paired contrasts are: (Medium - High) without mip, (Medium - High) with mip, (mip - no mip) under High, and (mip - no mip) under Medium. The difference of those differences is descriptive, not a causal estimate of user-visible fluidity.
- Explicitly exclude first-run UI startup from replay metrics while retaining separate all-frame cold-stage timing.

## Crucial cache-lifetime finding from first complete matrix

The 24-process exact-Windows run at 83b56a8 passed all quality/witness gates but used the *original short* `IMAGE_FRAMES=2` eviction policy. The 512-frame linger flag was absent, and each mip mode repeatedly reconstructed both levels during the same 4-second replay: 7 extra builds in compact 768×522; 6 in large 983×675. The summary previously labeled cumulative mip-build time as "Cold mip", but that was inaccurate: `metrics.frameStages` covers all frames, including those after replay begins. The benchmark now reports **whole-process mip time**, **replay-only mip time** and **replay rebuild event count** independently. The cached mip levels (L1/L2) are often absent between redraws, driving occasional ~3ms CPU frames even when average sampling is inexpensive.

This is evidence about the 2-frame *experimental* mip cache; it must not be generalized to a separate 512-frame-linger policy or the installed product. Comparing `High` vs `Medium` also changes interpolation quality; a 2×2 CPU factorial cannot approve visible image fidelity. See [full evidence review](https://github.com/Lalalalendia/Akula/pull/406#issuecomment-6277450584).

## Limitations / next gate

This trial changes both mip source resolution and image sampling *when mip is enabled*, so a true pure-policy cache experiment requires holding reducer and filter fixed. One repeated source is an insufficient 10/100-image cache-churn workload. A later Windows GUI pass must use **multiple independently hashed PNGs**, forced meaningful image-covered damage/full redraw and zoom changes, with image pixel-quality comparison at several scales. Preserve user-facing installed path and the production-fidelity gate.

No FPS/scanout claims; no new packages, unsafe code or production sampler activation.
