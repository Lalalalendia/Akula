# Native scroll input-to-CPU-frame A/B

Status: **diagnostic only**, no modification to render policy or release defaults.

## Question

PR #265 measured draw_frame() and found a favorable mixed-frame raster
crossover; that cannot be reported as the application's scroll FPS or total
input latency. This probe measures a larger *synchronous* part of the UI
pipeline, without attempting to measure the OS/compositor/display.

Two genuinely distinct production constructor configurations are compared:
- Fixed two workers: AKULA_VELLO_SINGLE_RECT=0 (actual rollback).
- Adaptive two workers: AKULA_VELLO_SINGLE_RECT=1 (current default).

Each Linux release-mode process loads a real Akula note into Masonry. It sends
real pointer scroll and animation events, calls RenderRoot::redraw() to build
the updated layout/scene, then draws into the native CPU framebuffer. Two
fixtures are included: plain text 1000 x 600 at 1x, and a book cover 1750 x
1051 at 1.75x. The mixed workload injects a full invalidation every eight
steps. Pointer-only idle steps are excluded from measured distributions.

The probe reports three timed stages plus their total, with p50, p95 and
sampled sums. It uses six independent test processes, alternating fixed and
adaptive launch order across three trials. Each scene checks natural pixel
moves, small damage, full-frame redraws, expected context switching,
and strict <=1/255 partial-versus-independent-full fidelity on six selected
frames. Event and damage geometry counts must match across independent
processes. Full framebuffer digests are retained for diagnostics but NOT
compared byte-for-byte across fresh temporary UI profiles: the first CI run
proved that even two independent fixed-worker sessions differ in their full
frame fingerprints despite passing internal full-frame fidelity checks.

## Run

Execute the Native scroll input-to-CPU-frame A/B diagnostic workflow. Its
raw logs and generated Markdown summary are retained as Actions artifacts.

## Interpretation

Do **not** equate these timings to end-to-end visible FPS. This test excludes
the host event queue, actual window surface upload, compositor, vsync,
scheduling and user-to-photon latency; it also warms the UI and excludes
startup/cold note load. Stage p95 values cannot be added together. No
numeric performance threshold is hardcoded into CI because Linux runner
contention can change timing.

If the full synchronous pipeline does not consistently benefit, stop
additional Vello micro-optimization and profile the larger non-render stages
before considering new production changes. The shipped threshold of 350,000
physical pixels, current worker policy and instant rollback are unchanged.
