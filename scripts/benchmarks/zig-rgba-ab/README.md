# Akula / isolated Rust vs Zig RGBA experiment (2026-10-22)

Purpose: compare two implementations of exactly one existing Akula kernel,
aquilum-app/native/src/render/paint/mipmap.rs::half, without modifying the
installed application or its CI paths.

Variants: Rust scalar, Zig scalar, Rust packed u32, Zig packed u32.
Zig is pinned to 0.17.0 with a SHA-256 verified official x86_64 Linux tarball;
Rust 1.99.1 is pinned by the one-off workflow. All variants use a caller-owned
output buffer, the same source byte array and the same rounding (sum+2)/4.
No early exit for alpha; both kernels compute the opacity hint.

Correctness: independent Python oracle on 13 small and irregular dimensions,
then byte-for-byte comparison on 1280x884, 1281x885, 4096x2160.
All input RGB values are constrained to be <= alpha.
The result metadata flag 0 (no mip), 1 (opaque), 2 (transparent)
is checked too. A parity mismatch aborts without performance claims.

Timing: 12 alternating forward/reverse rounds per large size,
discard 2 warmups; report 10 samples, median and p95.
Size and build time: shared library bytes and cold wall time.
Memory: separate peak RSS samples for one Python process per variant.
These include Python, the allocated input/output, and the library:
they are NOT pure kernel resident-memory readings.
Neither kernel allocates memory internally.

Do not extrapolate microbenchmark medians to full GUI FPS or total startup.
First compare the scalar versions, then packed ones. A change to the production
renderer requires a separate proven end-to-end test and explicit approval.

To run on a Linux x86_64 machine with Rust 1.99.1, Python 3.12 and curl:
    bash scripts/benchmarks/zig-rgba-ab/run.sh

Only the research branch has a one-shot, branch-scoped push workflow; no PR
is opened and no workflow or application file from main is modified.
