# IMG-PERF-09 — Causal A/B for image-mipmap cache lifetime

**Status: independent developer-only experiment, no installed behavior change.**

## Problem and evidence

[IMG-PERF-07 PR #406](https://github.com/Lalalalendia/Akula/pull/406) measured 24 native Win32 process instances separating Vello's High/bicubic and Medium/bilinear image interpolation from mipmap allocation. The Medium filter itself was faster in the stage instrumentation. But the original 2-frame mipmap lifetime (not 512-frame linger) made L1+L2 rebuild during the four-second scroll: 7 rebuilds per compact process and 6 per large process, with ~19ms total whole-process build CPU and recurring approximately 3ms costly frames. **The filter matrix does not independently prove that 512-frame retention eliminates this tail**.

## Controlled single-factor comparison

Use the *existing*, proven native-Windows harness with the same pinned 1280×884 city photo, original long 240-section Markdown note, the same uninstalled release binary and sampled viewport dimensions.

| Mode | Image sampler | Box reduction | Mip cache expiry |
|---|---|---|---|
| `mip_medium` | Medium | Original scalar 2×2 L1→L2 | 2 frames |
| `mip_medium_linger` | Medium | Original scalar 2×2 L1→L2 | 512 frames |

Both modes opt in to `AKULA_EXPERIMENTAL_IMAGE_MIP=1`, use the original 32 MiB budget, create the same pixel-identical mipmap levels and run the same four-second replay. The only intentional variant is `AKULA_EXPERIMENTAL_IMAGE_MIP_LINGER=1`. Experimental direct packed L2 must remain off.

Run 3 paired trials at each of 2 Win32-supported real-window sizes: **12 processes**, with AB/BA reversed in alternating trials. Witness positive mip use and an uncloaked HWND owned by the process; verify the exact executable SHA, note SHA and PNG SHA do not vary between modes.

## Fail-closed sample/algorithm contract

- Every row must provide all-frame and replay-only stage arrays aligned with the actual presented replay frames; values finite and nonnegative.
- Whole-process L1+L2 generation must occur at least once. Count **replay rebuild events** separately from original setup: a frame's `mipGenerated > 0` contributes one event; whole- and replay-generation levels must remain even for this simple two-level source.
- **Expected outcome to test, not assume**: short retention rebuilds at least once in replay; 512-frame linger rebuilds zero times in replay, because fewer than 512 frames are traversed in the same standard fixture. If either condition is false, the workflow correctly fails and the lifetime hypothesis must be revisited.
- Record replay-only CPU-work p95, p99, mean, number of rebuilt mip levels, replay-mip CPU time, whole-process mip CPU time, peak Windows working set, and per-trial paired deltas. Do not call whole-process build time *cold cost* when it includes replay.
- No FPS/scanout claim, no per-run arbitrary CPU-time performance threshold (shared CI load varies).

## Limits and next steps

Increasing cache lifetime could increase memory retention, and the optimal TTL is not automatically 512 frames. In a multi-image workload memory budget dominates and cache entries can be evicted even before TTL. A viable production policy should combine memory budget, priority by expected reuse, selected level and an idle/visibility grace period. That requires separate image-heavy full-redraw evidence ([IMG-PERF-08 PR #412](https://github.com/Lalalalendia/Akula/pull/412)) and image-quality validation, including HiDPI, alpha and zoom. Installed Akula has **mipmap disabled** and is not affected by these flags. No installed activation is proposed here.
