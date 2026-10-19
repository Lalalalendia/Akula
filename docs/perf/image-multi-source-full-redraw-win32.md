# IMG-PERF-08 — Native Windows multi-image, full-redraw LOD strategy stress

Status: **uninstalled developer-only**, not a production change. Builds on [IMG-PERF-06 PR #401](https://github.com/Lalalalendia/Akula/pull/401) merged as da7cbfe. The existing single-photo scroll benchmark is mostly a BGRA row-copy benchmark and is unable to expose the benefit of image sampling. This experiment forces real full raster during replay and uses many separately hashed PNG sources.

## Invariants

- Each source file is a genuinely distinct 1280×884 RGBA8 PNG, created deterministically by a pure-Python PNG writer using a compressed sequence of color/alpha bands and per-file seeds. Individual SHA-256 values must be unique; filenames by themselves are insufficient.
- Test fixtures contain **1, 10 and 100 distinct PNG source files** with all filenames embedded across a **240-section scrollable note**, plus repeated images to force actual repaint. A single image followed by one heading was insufficient to drive a valid replay in the first CI attempt (0 mip requests). Images display at 200px width so the normal mip level selector targets L2.
- Mode Eager: existing opt-in mipmap, Medium filtering, 512-frame cache lifetime, builds L1 and L2 using scalar 2×2 reduction.
- Mode Selective: **identical image content, viewport, Medium sampler, image source identifiers and 32-MiB budget**, but opt-in developer-only direct packed L2, without initial L1 allocation; lazy L1 remains possible.
- Developer-only \`AKULA_EXPERIMENTAL_IMAGE_FORCE_FULL_REDRAW=1\` forces a complete raster during an active replay, defeating optimization of copying existing BGRA rows. Positive \`AKULA_IMAGE_REPLAY_FULL_REDRAW_APPLIED\` witness is mandatory. Normal and installed rendering paths remain unchanged.
- Opt-in `AKULA_EXPERIMENTAL_IMAGE_DISTINCT_PROOF=1` records both **distinct runtime image IDs** and **distinct sampled decoded-pixel fingerprints**, reported at least every 32 frames. For each newly seen ID, BLAKE3 hashes dimensions and three complete decoded RGBA scanlines (first/center/last). That samples about 15 KiB per image rather than hashing 4.3 MiB during the measured frame; it is **not a complete file hash**. The generator's 100 seeded fixtures differ in these scanlines, and separate fixture SHA-256 still proves their original files differ. Require **both** counts to reach the threshold so repeated instances of one PNG cannot masquerade as different images.

## Direct-pixel scan admission after invalid smooth-wheel replay

[Exact-head run #39328178378](https://github.com/Lalalalendia/Akula/actions/runs/39328178378) confirmed native Win32 full-frame rasterization for the one-PNG case but then rejected the 10-PNG case: **3/10 decoded-pixel fingerprints**, stable from frames 32 through 320, below the unchanged required 8. Existing `--replay-fling` injects smooth mouse-wheel line events which the editor handles with a retargeted animated glide. That motion did not cover enough different image positions, even though 320+ replay frames existed.

The next **developer-only** attempt keeps exactly the same PNG contents, 240-section Markdown fixture, size, sampler, 32 MiB cache, expiry, and 6-second native fling schedule, but injects `ScrollDelta::PixelDelta` through the **same real Win32 Host pointer-event routing** when `AKULA_EXPERIMENTAL_IMAGE_SCROLL_SCAN=1`: a 240 logical-pixel jump per replay wheel step (down in the first half, back up in the second). This removes *glide smoothing* solely from the image stress workload, not from ordinary Akula. A mandatory `AKULA_IMAGE_SCROLL_SCAN_APPLIED logical_step=240` witness records real use. The changed scroll trajectory applies identically to Eager and Selective, with unchanged sampling and rendering.

This is not a claim that real users scroll at 240 px per event. It is a controlled, deliberately high-coverage **stress scan** intended to force decoding and rendering many distinct sources within a fixed six-second run. Require actual per-source mip requests, independent decoded-pixel fingerprints, and all-frame dirty-pixel area rather than relying on filenames or inferred traversal. If the thresholds still fail, inspect editor content length, viewport coverage or lazy loading; **do not lower the minimum image counts**.

## Acceptance

One native Windows release binary, visible uncloaked PID-owned Win32 HWND and monitored peak working set. For each of 1/10/100 source-count scenes, run two process-level Eager/Selective pairs (12 total) in alternating AB/BA order, each with a native six-second fling replay. Record:

- original fixture hashes, number of source IDs **actually requested by mipmap**, positive mip and full-raster witnesses;
- replay-only CPU-work p95/p99/mean and aligned per-replay renderer stage metrics;
- all-frame and replay-only mip build costs, total generated levels, cache hits/evictions/retained levels;
- peak process RSS, median fully damaged raster pixel area, Win32 window dimensions and exact binary SHA;
- paired difference in p95. Never interpret CPU work as physical display presentation or user-visible FPS.

Minimum valid source coverage **for both runtime IDs and pixel fingerprints**: scene 1 → **1** used, scene 10 → **8** used, scene 100 → **24** used. Why 24? For 1280×884 L1+L2, 24×1,414,400 bytes exceeds the old 32 MiB cache, so this scene actually challenges retention. The 100-file scenario must not pass by displaying only the first picture. A 24-image unique access count is not proof that 24 are simultaneously resident; compare evictions, timing and memory honestly.

All sample files are pure-SHA deterministic and require no external Python packages. GUI startup and loading are not excluded from the whole-process stage array but **are excluded** from replay p95/p99. More trials are necessary if two pairs disagree.

## Attribution caveat

This compares **combined** Eager-scalar-pyramid vs Selective-packed-direct-L2-and-lazy-L1. Even with identical Medium sampling and rendered pixels, differences in cold latency cannot be attributed solely to caching policy: the reduction algorithm also differs. A pure policy-only comparison must hold the reducer constant (packed eager vs packed direct). Keep that for a separate factorial study. This PR neither changes the installed renderer nor proposes automatic mipmap rollout. Pixel-fidelity screenshots/pattern checks are still required before production approval.
