# Akula: developer-only mipmap row reducer in the actual native renderer

The Zig-vs-Rust proof (#39706244494) found the row-oriented Rust kernel
faster than the original Zig packed kernel on its isolated test buffers.
This does NOT establish any user-visible rendering advantage.

This branch adds an explicit opt-in to Akula's EXISTING experimental image
mipmap path. The installed build is unchanged (cfg(feature="installed")
does not include mipmap cache, reducer selector, or any environment logic).

To try the row reducer, start an UNINSTALLED developer executable with BOTH:
    AKULA_EXPERIMENTAL_IMAGE_MIP=1
    AKULA_EXPERIMENTAL_IMAGE_MIP_HALF_ROW=1

Without the second flag the old scalar reducer remains intact. Direct L2,
if explicitly selected, bypasses the row half-reducer and is NOT compared.
The benchmark forces mip_high, disables direct L2, and verifies the
AKULA_MIP_HALF_ROW_USED log marker only for the row processes.

Only complete and validated Pixmap buffers are read. The RGBA alpha flag,
premultiplied representation, source dimensions and clamping are preserved.
Tests compare RGBA bytes for odd/thin and opaque/mixed images; tests also
compare FULL Vello-rendered pixel buffers over varying ratios, quality,
phase and worker count.

The branch-only one-workflow/two-jobs lab runs actual Win32/Winit and
Linux Xvfb/Winit windows using the existing native window harness.
Three alternating paired trials per OS, two modes in the same binary,
positive mip-use and HWND witnesses, exact input SHA, frame-stage metrics,
peak RSS, replay-only CPU work and actual mip-generation timing.

No production rollout, feature default, policy gate or main workflow change.
No PR should be opened just because the diagnostic passes. The target
metric is a genuine improvement of full editor workloads and cold build,
not merely isolated kernel speed. Cached-image replay may show no speedup.

## 22 Oct — Windows city-cover Vello and multi-image test

A Windows-only branch-scoped push now runs byte parity over 65535x1 and 1x65535
pixel limits, actual city-cover PNG resized to 257x193 with premultiplied opaque
and alpha-cropped imagery, full Vello output under three downscale ratios and
two sampler/worker settings, and 1/10/100 separate image-ID L1/L2 cache budget
and direct L2 equivalence. Existing rounded-corner, alpha-crop and fractional
pixel tests also run under Windows. These are test-only changes; no installed
binary, production default, Linux-tested algorithm or main CI was modified.
