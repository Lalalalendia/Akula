# IMG-PERF-05 — Direct L2 without intermediate L1 allocation

Status: test-only experiment, not a change to the renderer. Previous evidence: [PR #390](https://github.com/Lalalalendia/Akula/pull/390) demonstrated 2.32–2.40x speedup for a **single** mip reduction by a byte-exact 32-bit packed RGBA approach.

## Exact equivalence requirement

For each destination pixel in mip level L2, compute four *virtual* L1 pixels, each as a 2×2 average of original RGBA8 texels using `(sum+2)/4`. Average the four L1 results with the **same second rounded** `(sum+2)/4`. Clamp original coordinates for L1 sampling and separately clamp L1 coordinates when L2 samples a padded odd edge. **Do not** replace the two-stage roundings with a single 4×4 average or an unrounded 16-texel sum.

This direct L2 result should be byte-for-byte equal to `half(half(src))`, including RGBA values and opacity metadata. Test 1×1 / 1×N / N×1, all shapes 1..11, alpha-zero/premultiplied colors, even and odd 1280×884/1281×885 examples.

## Controlled CPU comparison

In release-mode Windows CI, the opt-in ignored test measures the three independent strategies on identical source bytes: scalar L1→L2, packed L1→L2, and direct packed L2 with no retained L1. Twelve measured repetitions per mode and size, plus two warmups, all six order permutations cycled. Sizes 1280×884, 1281×885, 4096×2160. Emit `AKULA_MIP_B2_MICROBENCH` lines. No speed threshold on shared CI runners.

Cost model on 1280×884: the packed two-pass path materializes a 640×442 L1 (**1,131,520 bytes**) and then a 320×221 L2 (**282,880 bytes**). The direct path materializes only L2, removing L1 allocation, storage and its subsequent reread. This reduces transient memory **if L1 is not independently required**; duplicate L1 samples and address calculations may still negate the speed improvement.

**Important ownership rule:** Akula's current `Images.mips` retention policy stores intermediate L1 intentionally, including for future zoom levels. This B2 test-only candidate does not alter that cache; a production selective-direct-LOD policy must separately guarantee future L1 availability, exact identity/version, alpha and fidelity, eviction accounting and zoom reuse. Direct L2 cannot replace the general pyramid unconditionally.

No installation change, no unsafe instruction, no feature flag, no new dependencies. The score is a microbenchmark, not end-user frame pacing.
