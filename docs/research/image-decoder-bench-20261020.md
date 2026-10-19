# IMG-DEEP-04 — measured JPEG decoder comparison

Research artifact, not shipping code. Reproducible harness bundle: `akula-image-bench.zip` in the ChatGPT Library, recorded in the linked [Notion research](https://app.notion.com/p/3fe32a84beec81a89790cf45c29e3d56).

## Verified local result — libjpeg only

On the current Linux environment, the compiled scanline libjpeg baseline decoded a 4096×3072 JPEG (about 6.9 MB input) to 1600×1200 RGBA:

| Format | Internal read+decode | Max RSS (GNU time) | Output |
|---|---:|---:|---:|
| Baseline JPEG | 258.568 ms | 8,448 KiB | 7,680,000 bytes |
| Progressive JPEG | 427.132 ms | 45,312 KiB | 7,680,000 bytes |

These are single-run smoke measurements, *not* a comparative performance result. The progressive variant was encoded from the first JPEG, not identical image-file contents. Machine/toolchain parameters were not benchmark-controlled. No Rust or libvips run was possible in this environment. The original 32 MiB cap in PR #403 is unchanged.

## Source-audit observations

- `aquilum-app/native/src/images/decode.rs`: JPEG uses scaled `jpeg_decoder::Decoder::decode()`, materializes scaled intermediate RGB before RGBA; normal PNG uses `next_row`; other format fallback may fully decode before reduction.
- `aquilum-app/native/src/images/mod.rs`: `fs::read` buffers full compressed input.
- `aquilum-app/native/src/render/pixels.rs`: transfers owned RGBA `Vec` into `Pixmap::from_parts`, with no obvious full-array clone at that boundary.
- `libjpeg-turbo/src/jdapistd.c`: scanline output, constrained crop and skip; progressive input may still require a whole-image DCT coefficient store.
- `libvips/foreign/jpeg2vips.c`: 8-scanline sequential strips; `libvips/resample/thumbnail.c`: pre-shrink followed by quality filtering, ICC/EXIF handling.
- `OpenImageIO/src/libtexture/imagecache.cpp`: native vs virtual tiles and soft resource limits; avoid unbounded automip cache sizing.

## Acceptance still needed

CI must compile the Rust harness with Akula's exact decoder source and dependencies, compile the libjpeg-turbo and libvips runners, run identical deterministic baseline/progressive images, capture whole-process peak memory + wall time and RGB error, and archive results. Repeat on physical Windows hardware separately. Compare *quality* separately; output matching is not a proof of visual fidelity. Code is research-only; do not auto-merge or change the production decoder.
