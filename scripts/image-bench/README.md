# Three-way JPEG decoder probe

A reproducible standalone harness has been prepared alongside [IMG-DEEP-04](../../docs/research/image-decoder-bench-20261020.md). The full source package, including the Rust decoder harness using the real `aquilum-app/native/src/images/decode.rs`, C scanline libjpeg benchmark, C libvips thumbnail benchmark, deterministic fixture generator, and CI workflow, is available as `akula-image-bench.zip` in the linked Notion research archive / conversation deliverable.

The archived bundle has not yet passed full Rust and libvips CI. This directory intentionally **does not pretend** a completed or passing benchmark is available in main. The local C libjpeg mode compiled and decoded both baseline and progressive JPEG. Next action: import the reviewed complete archive into this branch and use its `run.sh` for Ubuntu CI before any production changes.
