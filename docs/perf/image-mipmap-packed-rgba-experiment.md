# IMG-PERF-04 — Test-only SWAR 2x2 RGBA box reduction

**Purpose.** Evaluate byte-exact unsigned 32-bit packed arithmetic against existing scalar per-channel 2x2 RGBA reduction in Akula, without changing any production renderer or cache behavior. Parent analysis: PR #386 and its 12 native Windows stage profiles (3.52/4.10ms median cold build, no replay rebuilds).

The baseline `half()` reads four premultiplied RGBA8 texels per output pixel and averages each channel with `(sum+2)/4`. The candidate `half_packed_trial()`, available only under `#[cfg(test)]`, interprets each pixel in little-endian 32-bit layout, accumulates R/B and G/A separately in 16-bit lanes, adds +2 per lane, divides by four, and reconstructs the four original 8-bit channels. Every lane sum is bounded by 1020; even with rounding it cannot carry into its adjacent 16-bit lane.

## Correctness gate

Release tests compare the exact byte buffers, dimensions, and opacity hint of baseline vs candidate for 1×1, thin 1×N/N×1, even and odd sizes, repeated right/bottom edges, opaque and mixed-alpha premultiplied input, and four deterministic seeds. A no-change `None` result is required for 1×1. This is **pixel-identical**, not gamma-corrected or a new filter.

## CPU experiment

The Windows native mipmap platform QA workflow runs a non-gating release-mode microbenchmark, with a pinned pseudorandom premultiplied image at 1280×884, 1281×885 and 4096×2160. Twelve alternating AB/BA iterations, first two discarded, medians reported in the log as `AKULA_MIP_B1_MICROBENCH`. No arbitrary speed thresholds; results are advisory and on shared CI hardware.

If packed scalar is slower or equivalent, do **not** merge it into the installed image pipeline. If faster, require additional real GUI cold-frame and image-fidelity A/B before any opt-in or production activation. As input staging/cache and execution differ on the user's Windows machine, these numbers are not application frame time or wall-clock present latency.

## Scope

`mipmap::half` is **unchanged**. No release flags, no new dependencies, no unsafe code, no image-quality changes; all new reducer code compiles only in Rust tests. Study direct L2, SIMD and asynchronous generation in separate experiments.
