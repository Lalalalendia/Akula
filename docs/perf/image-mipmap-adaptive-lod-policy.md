# IMG-PERF-06 — Selective L2 with lazy L1: bounded cache experiment

Status: **experimental, developer-only**. Previous [PR #392](https://github.com/Lalalalendia/Akula/pull/392) established byte-exact direct L2: on hosted Windows, approximately 3.3–3.5x faster than the original two-pass scalar renderer. This PR applies that validated computation only behind an explicit developer flag. Installed Akula cannot enable it.

## Actual policy switch

Requires **both** `AKULA_EXPERIMENTAL_IMAGE_MIP=1` and `AKULA_EXPERIMENTAL_IMAGE_MIP_DIRECT_L2=1`. This is evaluated once per process. Absent the second flag, the experimental mip cache uses its existing eager L1+L2 policy unchanged. The `installed` build never includes the mip module or reads either flag.

When a final L2 is requested:
1. Reuse its existing cached `(image_id, 2)` entry immediately if present.
2. If `(image_id, 1)` is already present, use the existing incremental build path (no rebuild of L1).
3. If both are missing and direct-L2 is selected, reserve **only** L2 bytes within the current 32 MiB cap, evicting only other-source levels by LRU if necessary. Rejected reservations change nothing.
4. Compute L2 from original premultiplied RGBA with the two byte-exact rounded 2x reductions performed virtually, **without materializing L1**. Cache L2 by the existing source-ID/level key, preserving the same width, height, alpha metadata and byte payload.
5. On a later request for L1, compute it from the immutable original and reserve space as usual. A subsequent L2 request must reuse the same Arc rather than silently replace an already valid L2.

The code intentionally handles exactly `level==2` as the new direct case. Higher mip levels still follow the existing approach; source identity semantics and max limits do not change.

## Correctness and stress tests

The Windows release test suite compares eager vs direct RGBA output, and covers:
- Exact original-to-L2 pixel parity and premultiplied alpha/opacity hint.
- Final L2 pointer reuse, delayed L1 correctness and preservation of the existing L2 on zoom.
- Strict failed-reservation/no-partial-cache mutation, including a later L1 request that cannot fit next to resident L2.
- **1 / 10 / 100 distinct** deterministic RGBA inputs (256×192), under a deliberate **147,456-byte** stress budget. Compare resulting pixel bytes and inspect resident levels, eviction and zoom revisits. A full pyramid occupies 61,440 bytes per input while L2-only occupies 12,288 bytes, so the test must force different residency patterns.
- An optional non-gating release-mode microbenchmark separately times first access and subsequent revisit for the eager and selective paths on the same input sequences.

No speed threshold is used to gate CI: hosted machine variability is expected. `source` and test output bytes are deterministic; production image quality and system-level frame pacing require their own checks.

## Byte economics

At 1280×884 RGBA, L1 = 1,131,520 bytes, L2 = 282,880 bytes. On a first L2 request, the selective path avoids retaining L1 (1.08 MiB per image), and prior single-input Windows microbenchmarks show a cold-build saving of about 0.55 ms compared to packed L1→L2. A late L1 request costs another ~1.13 ms on that runner. The simplified crossover is ~48.5% L2-first images later needing L1; this is a **model, not observed user behavior**.

Expected improvement is at cold image render / mixed-image pressure, not ordinary scroll on a largely reused BGRA framebuffer. Before proposing any installed-path change, measure: true L2-first→L1 transitions in real documents, source identity/revision turnover, bounded 32 MiB cache churn, full redraw / zoom fidelity and per-frame worst-tail latency on Windows. Maintain the existing production-fidelity gate.

## Out of scope

No change to installer, default Vello image quality, installed renderer, GPU scanout or the image/markdown document model. No unsafe code or new external dependencies. This PR is a falsifiable developer experiment, **not** approval to enable mipmaps by default.
