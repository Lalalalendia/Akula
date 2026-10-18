# Akula — opt-in adaptive Vello for a single dirty rectangle

Status: **experimental, disabled by default**. This is a narrow continuation of the
merged [#166](https://github.com/Lalalalendia/Akula/pull/166) diagnostic and
[#195](https://github.com/Lalalalendia/Akula/pull/195) natural-damage profiling.
Do not duplicate existing Vello/glyph cache or damaged-region optimizations.

## Proven existing facts

- The natural Akula scroll suite has 41 scroll-candidate frames per scene,
  `max raster_rects=1`, 0 multi-rectangle frames.
- On the Linux runner, serial was fastest for small natural damage. In a
  cover/1.75× sample, reset p50 with 0/2/4 workers was 1.3/38.1/70.1 µs.
  These are *stage* timings, not overall application frame times.
- Test-only dual routing in #166 saved roughly 1.4–13% on one Linux mixed
  scroll/full run; that is insufficient evidence for broad default enablement.
- Synthetic 2/4/8/16-strip studies in #173/#187 did not justify a frame-area
  threshold or coalescing for ordinary scrolling. #187 captured a >1-LSB
  cross-partition pixel mismatch. #179 independently tracks an intermittent
  guard96 >1-LSB mismatch.

## Deliberately narrow implementation

- Set `AKULA_EXPERIMENTAL_VELLO_SINGLE_RECT=1` **before starting Akula** to
  enable this route for a renderer whose normal worker count is nonzero.
- Keep a persistent 0-worker `RenderContext` alongside the normal context,
  without rebuilding worker pools at each redraw. Both contexts share a single
  `Resources` object (glyph atlas/image registry/outline cache) and image/mask
  caches in `Painter`.
- Decide **once per final frame damage list**, after live-layer updates and
  before encoding. Serial only for exactly one **non-empty** rectangle whose
  area is ≤350,000 physical pixels. Any zero, two or more, or larger dirty
  areas retain the original parallel renderer. This threshold is a provisional
  experiment parameter, *not* a validated universal optimum.
- `Painter::trim` recreates both contexts with correct worker counts, and
  restores parallel as the primary.
- With the env flag unset (default), no extra serial context is allocated and
  the existing worker policy is unchanged.

## Go/no-go

1. Required tests: direct single-region / fragmented / trim / disabled checks.
2. Real Akula mixed scroll/full 0/2/routed pixel equality on every frame,
   independent full reference ≤1/255. Linux three-trial A/B and Windows/macOS
   real frame parity required before shipping.
3. Record peak process RSS **on the same workload**, with and without the
   experimental mode; cap incremental memory before enabling by default.
4. Guard96 #179 and cross-partition issue #187 remain separate correctness
   gates. Do not loosen pixel tolerance or disable hinting/atlas.
5. Repeated release-mode A/B: keep only when mixed workload totals and p95
   improve consistently against the best viable fixed policy; otherwise leave
   opt-in disabled or remove it.

This change intentionally does not add a generic region merger, new glyph cache,
alternative draw algorithm, or background resource scheduler.
