# Akula real-kernel port comparison

Four native compiler variants are checked against *actual algorithms* (standalone ports) from Akula source at SHA 641a437005132a4520b3606586d11d112f19fd5a.

- RGBA mipmap: aquilum-app/native/src/render/paint/mipmap.rs half. Premultiplied-alpha, edge padding for odd dimensions, rounded 2x2 average and opacity flag.
- Raster dilation: aquilum-app/native/src/graph/raster/mod.rs dilate. Separable 3x3 maximum with edge replication.
- Byte delta: aquilum-app/core/src/sync/text_delta_codec.rs splice and splice_apply. This is only the byte core, not compression, signature, revision validation, publishing or actual synchronization.
- Text matcher: aquilum-app/core/src/text/matcher/mod.rs Matcher::each, **only Mode::Substring + Fold::Exact** on valid UTF-8. Full text search functionality is broader.

Candidates: C, C++ compiler applied to the exact same C-compatible code (not idiomatic C++), Rust and Zig 0.17. The separate prior generic benchmark tested 12 languages, including Nim, D, Pascal, Fortran, Odin, C3, Go and Python. It is *not valid* to claim those languages were measured on these Akula kernels.

Harness: run.py loads four native C ABI libraries with ctypes. All small cases are compared exactly against independent Python reference functions. Six larger cases are compared byte-for-byte to a verified C reference (the delta patch compared to Python even on large inputs). Timing uses warmups and 12 shuffled rounds, each approximately 40 milliseconds with preallocated inputs/outputs. Outputs are JSON, CSV and a Markdown summary. Per-language memory cannot be isolated because the benchmark shares a process: no invented RSS comparison.

Artifacts require every candidate to compile and pass parity. No Akula production or main files changed. Further work: actual Matcher folding, end-to-end UI, bigger and more heterogeneous workloads, CPU pinning and cross-platform testing.