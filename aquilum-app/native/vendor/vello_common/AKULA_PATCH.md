# Akula's Vello Common 0.3.0 patch

The source files (apart from this note and the documented single-line rounding change) are taken from the published `vello_common 0.3.0` crate verified against `aquilum-app/Cargo.lock`, sha256 `73c2fbe53548bcd25c8ff42f75a93770a8611a16b37bee56d7ef7b44e36027f1`.

The upstream crate is dual licensed `Apache-2.0 OR MIT`; both original licenses are retained. This local path override keeps patched code reproducible under `cargo --locked`, including offline builds. Do not silently upgrade Vello Common without rerunning the strict reference gate.

**Difference:** `src/strip.rs` NonZero alpha coverage `f32x4::splat(s, 0.5)` becomes `f32x4::splat(s, 0.5001)`. The tiny alpha-unit tolerance resolves the demonstrated exact half-tie (ideal `255*5/6=212.5`) where float32 strip arithmetic varies with atlas slot/tile origin. No glyph geometry or filter is changed.

**Evidence:** isolated real glyph #495 (11×9, five atlas slots full/crop); strict `guard96` full frames 20/20 Linux, 5/5 Windows and macOS, [cross-platform run](https://github.com/Lalalalendia/Akula/actions/runs/39125361673); [clean full-frame A/B](https://github.com/Lalalalendia/Akula/actions/runs/39125361683) found 32/9,757,000 changed pixels, none over 1/255. The old conversion was falsified at this exact glyph. Further fonts/scale coverage and upstreaming are desirable; this is an app-local stability change, not a blanket upstream correctness proof.
