# IMG-PERF-08 — Native Windows multi-image, full-redraw LOD strategy stress

Status: **uninstalled developer-only**, not a production change. Builds on [IMG-PERF-06 PR #401](https://github.com/Lalalalendia/Akula/pull/401) merged as da7cbfe. The existing single-photo scroll benchmark is mostly a BGRA row-copy benchmark and is unable to expose the benefit of image sampling. This experiment forces real full raster during replay and uses many separately hashed PNG sources.

## Invariants

- Each source file is a genuinely distinct 1280×884 RGBA8 PNG, created deterministically by a pure-Python PNG writer using a compressed sequence of color/alpha bands and per-file seeds. Individual SHA-256 values must be unique; filenames by themselves are insufficient.
- Test fixtures contain **1, 10 and 100 distinct PNG source files** with all filenames embedded across a **240-section scrollable note**, plus repeated images to force actual repaint. A single image followed by one heading was insufficient to drive a valid replay in the first CI attempt (0 mip requests). Images display at 200px width so the normal mip level selector targets L2.
- Mode Eager: existing opt-in mipmap, Medium filtering, 512-frame cache lifetime, builds L1 and L2 using scalar 2×2 reduction.
- Mode Selective: **identical image content, viewport, Medium sampler, image source identifiers and 32-MiB budget**, but opt-in developer-only direct packed L2, without initial L1 allocation; lazy L1 remains possible.
- Developer-only \`AKULA_EXPERIMENTAL_IMAGE_FORCE_FULL_REDRAW=1\` forces a complete raster during an active replay, defeating optimization of copying existing BGRA rows. Positive \`AKULA_IMAGE_REPLAY_FULL_REDRAW_APPLIED\` witness is mandatory. Normal and installed rendering paths remain unchanged.
- Opt-in `AKULA_EXPERIMENTAL_IMAGE_DISTINCT_PROOF=1` records both **distinct runtime image IDs** and **distinct sampled decoded-pixel fingerprints**, reported at least every 32 frames. For each newly seen ID, BLAKE3 hashes dimensions and three complete decoded RGBA scanlines (first/center/last). That samples about 15 KiB per image rather than hashing 4.3 MiB during the measured frame; it is **not a complete file hash**. The generator's 100 seeded fixtures differ in these scanlines, and separate fixture SHA-256 still proves their original files differ. Require **both** counts to reach the threshold so repeated instances of one PNG cannot masquerade as different images.

## Real image-anchor scan after two invalid wheel coverage attempts

The original smooth-wheel and the follow-up 240-pixel `PixelDelta` full-redraw Windows [run #39342283980](https://github.com/Lalalalendia/Akula/actions/runs/39342283980) each displayed only **3 distinct pixel contents out of 10 PNG files**, despite a 6-second replay, positive redraw witness and more than 300 timed frames. This proves neither a cache-policy failure nor a valid many-source stress test. Wheel motion alone does not guarantee asynchronously decoded, paint-visible images.

The new **developer-only byte-anchor scan** lists the UTF-8 start offsets of `![200](photo_NNN.png)` paragraphs in the actual note and sends those anchors to the existing editor's `top_of(byte)` and `App::reveal(root,byte)` path, through the **same native render root**. The six-second fling clock is divided into two three-second passes: visit up to **32 distinct image anchors** in forward order, then reverse. Each anchor remains the target for **3000 / min(N,32) milliseconds** (300ms for 10, ≈93.75ms for 100). No fake render calls, skipped decoders or new sample clock. The positive `AKULA_IMAGE_ANCHOR_SCAN_APPLIED count=N schedule=forward-reverse` witness is required; a missing note layout or failed seek cannot emit the positive witness. Logged seek positions are diagnostic only — acceptance still requires actual different decoded pixels to reach 1 / 8 / 24.

Eager and Selective use the exact same anchors, fixture hashes, timetable, Medium sampler, 32 MiB budget and 512-frame TTL. This is a deliberately artificial **coverage stress scan**, not a claim about ordinary human scrolling or physical FPS. Some images may never decode before the next seek; if the strict thresholds still fail, inspect editor lazy/decode timing and per-image visibility instead of lowering them. The restored default fling path is unchanged when the developer scan variable is absent.

## Four-stage source reachability proof

[The exact-head native Windows run #39351030514](https://github.com/Lalalalendia/Akula/actions/runs/39351030514) proved that *all ten* synthetic `photo_NNN.png` sources reached the asynchronous resolver in both `Pending` and `Ready` states. Despite this, the mipmap raster saw only **two distinct decoded pixel fingerprints**. The experiment now distinguishes four checkpoints, for **each** distinct source rather than merely counting PNG files:

1. `AKULA_IMAGE_RESOLVE_STATE ... pending`: image paragraph asked for asynchronous media decode.
2. `AKULA_IMAGE_RESOLVE_STATE ... ready`: usable decoded image pixels returned.
3. `AKULA_IMAGE_EMBED_PAINT ... ready=true`: an actual `MediaView` with ready pixels was visited by the live editor painter. A ready-but-never-painted source must not be counted as an LOD cache load.
4. `AKULA_MIP_CACHE ... distinct_sampled_pixel_fingerprints=N`: the actual Vello mip sampling path received different source pixels and built or hit an appropriate LOD.

The paint checkpoint is developer-only, active only for artificial `photo_NNN.png` fixtures and an explicit opt-in flag. No user path/content is logged. If the resolver Ready count is high but MediaView paint coverage is low, investigate editor virtualization, layout invalidation, stale scroll transforms and visibility clipping. If the MediaView count is high but Vello mip coverage low, investigate clip/raster sampling and dimensions. The **unchanged** strict 1/8/24 final decoded-pixel fingerprints remain the real admission; none of these preliminary stages can substitute for actual paint work.

## Render command invalidation and CPU raster culling falsifier

[Five-stage Win32 evidence #39378075009](https://github.com/Lalalalendia/Akula/actions/runs/39378075009), [raw process archive](https://github.com/Lalalalendia/Akula/actions/runs/39378075009/artifacts/12244482487): all **10/10** synthetic PNGs pass async Pending→Ready, visible editor paragraphs and materialized embeds, and **all ten actual `MediaView::paint` calls have non-empty images**. The artificial anchor scan reaches 3318px scroll offset; the Vello CPU mip path sees only **2/10** distinct source pixel fingerprints and no evictions. Thus the remaining gap is between a *recorded draw_image call* and the final CPU raster candidate; do not blame missing files, decode, editor virtualization or cache pressure.

The next test includes **two independent developer-only changes**, neither installed:
- On the real editor-byte anchor transition, invalidate `TextEditor`'s retained painting via `WidgetMut.ctx.request_render()`, in addition to `ScrollArea`'s normal layout/compose/post-paint. This tests whether only moving a retained scene leaves new-image draw commands clipped to old recording conditions.
- Join each synthetic `MediaView` brush's **actual image data ID** with the same ID observed at the Akula Vello CPU `PaintSink::fill` boundary. Record the draw's transformed bounds, raster frame rectangle, offscreen state and sprite state **once per source and state**. Distinguish a command discarded by clipping, an image brush substituted by sprite policy and a command surviving CPU culling but not reaching a mip level. These IDs and frame bounds are developer-only and contain no user filenames or pixel data.

If the repaint fixes true 8/10 and 24/100 decoded-source coverage, we have isolated a retained-command invalidation dependency for this synthetic scan. If it does not, compare the sink ID/bounds logs to the source/ready and actual mip counts, then investigate clip stacks and transform propagation. This run is **not** a production scrolling behavior guarantee; preserve pixel/quality/memory review before enabling any LOD optimization.

## Post-decode layout drift: anchored seek must converge, not fire once

Reinspection of [failed native Win32 trace #39378075009](https://github.com/Lalalalendia/Akula/actions/runs/39378075009/artifacts/12244482487) exposed a test-driver flaw not distinguishable from the source/paint counts alone. The tenth image's **real frame begins at document y=3515.8**, but the `TextEditor::do_paint` viewport covers only **y=2533..3208**. Its MediaView was recorded during the editor's expanded prefetch/paint range, but the CPU renderer **correctly** does not sample an image 308px below the viewport. A `MediaView::paint` call is not visible-pixel coverage proof. The earlier 10/10 MediaView → 2/10 mip count is therefore *consistent with proper clipping*, not evidence of a Vello regression.

The original workload invoked `App::reveal(root, byte)` **only once** upon switching anchors. But async image readiness changes measured paragraph heights and shifts image positions **after** that reveal. A single byte position computed against the old layout can point offscreen. The corrected native driver preserves its original six-second forward-and-reverse 1/10/32-anchor schedule and 8ms ticks, but remembers the currently active image anchor. During dwell it checks `TextEditor::top_of(byte)` against the current scroll/document transform and re-centers *only if* the target is more than **64 logical pixels** away from viewport center. No unnecessary relayout if already centered. The first/last scroll-limited images are allowed a central 128px band, while the much larger 300px+ misplacement is rejected.

Positive, once-per-actual-byte evidence `AKULA_IMAGE_ANCHOR_ALIGNED byte=N center_error=...` proves real post-layout positioning; `AKULA_IMAGE_ANCHOR_RECENTER` records readjustment. Before admitting any p95/p99/LOD readings, the Python runner requires **1/1, 8/10 or 24/32 distinct image byte anchors actually aligned after current-layout measurement**. Separately, it still requires 1/8/24 *distinct decoded source pixel fingerprints actually used by CPU mipmap*. These are deliberately independent gates. If all anchors align yet mip sampling still covers only two PNGs, inspect CPU scene clipping/bounds from the `AKULA_IMAGE_RASTER_FILL` trace; do not relax fidelity or source coverage criteria.

## Acceptance

One native Windows release binary, visible uncloaked PID-owned Win32 HWND and monitored peak working set. For each of 1/10/100 source-count scenes, run two process-level Eager/Selective pairs (12 total) in alternating AB/BA order, each with a native six-second forward-reverse image-anchor replay. Record:

- original fixture hashes, number of source IDs **actually requested by mipmap**, positive mip and full-raster witnesses;
- replay-only CPU-work p95/p99/mean and aligned per-replay renderer stage metrics;
- all-frame and replay-only mip build costs, total generated levels, cache hits/evictions/retained levels;
- peak process RSS, median fully damaged raster pixel area, Win32 window dimensions and exact binary SHA;
- paired difference in p95. Never interpret CPU work as physical display presentation or user-visible FPS.

Minimum valid source coverage **for both runtime IDs and pixel fingerprints**: scene 1 → **1** used, scene 10 → **8** used, scene 100 → **24** used. Why 24? For 1280×884 L1+L2, 24×1,414,400 bytes exceeds the old 32 MiB cache, so this scene actually challenges retention. The 100-file scenario must not pass by displaying only the first picture. A 24-image unique access count is not proof that 24 are simultaneously resident; compare evictions, timing and memory honestly.

All sample files are pure-SHA deterministic and require no external Python packages. GUI startup and loading are not excluded from the whole-process stage array but **are excluded** from replay p95/p99. More trials are necessary if two pairs disagree.

## Attribution caveat

This compares **combined** Eager-scalar-pyramid vs Selective-packed-direct-L2-and-lazy-L1. Even with identical Medium sampling and rendered pixels, differences in cold latency cannot be attributed solely to caching policy: the reduction algorithm also differs. A pure policy-only comparison must hold the reducer constant (packed eager vs packed direct). Keep that for a separate factorial study. This PR neither changes the installed renderer nor proposes automatic mipmap rollout. Pixel-fidelity screenshots/pattern checks are still required before production approval.
