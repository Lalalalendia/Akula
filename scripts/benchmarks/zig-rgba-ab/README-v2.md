# RGBA Rust/Zig code-generation comparison: stage 2

The previous single-host microbenchmark showed the specific Zig packed kernel
about 18-19% faster than Rust packed. No general language conclusion follows.

This follow-up compares: (1) original Rust packed, (2) Rust 32-bit unaligned
loads/stores, (3) Rust row-stride-hoisted loops, and (4) unchanged Zig packed.
Each is compiled for generic x86-64 and for the native GitHub runner CPU.

The emitted assembly of each function is saved with a static instruction
summary. This is evidence of machine code structure, not instruction throughput
or dynamic execution count.

Parity: all variants must produce identical RGBA bytes and the opacity flag.
13 small shapes use the independent Python oracle. Additional tests include
fully opaque, transparent and odd-edge fixtures. Large shapes compare against
original Rust scalar; no production code is modified.

Timing: 5 independent Python processes, 12 rounds in each, 2 warmups then
10 paired measurements, rotated AB/BA ordering. The headline result is the
median of the five process-level medians and the per-process ratio range.
Different CPU targets must not be compared across tiers as language evidence.

All compilation is isolated under scripts/benchmarks/zig-rgba-ab and the
research branch. One push triggers one optional workflow; no PR is created,
no installation / GUI test is added, and nothing is merged into main.
