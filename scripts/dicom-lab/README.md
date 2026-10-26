# Akula DICOM isolated research lab

This branch is a **research harness**, not product DICOM support or a clinical viewer.

Run the dedicated [workflow](../../.github/workflows/dicom-isolated-lab.yml)
on \`lab/dicom-rust-compare-20261026\`. The workflow deliberately is not
attached to the main app Cargo workspace or the default branch's PR gates.

Three independent standalone Rust packages:
- DICOM-rs 0.10.0 (MIT OR Apache-2.0), pure native decoder features.
- DCMfx 0.52.1 from pinned upstream commit 74246499134f85003ec6e50286377ddc55565d0c
  (AGPLv3: license evaluation required before any app integration).
- dicom-toolkit 0.5.0 (MIT OR Apache-2.0).

Only synthetic data, no patient information. \`generate.py\` creates nine
valid DICOMs and one truncated file. \`verify.py\` checks basic structure and
whole-file hashes, **not** DICOM standard conformance.

\`reference_check.py\` uses pydicom 3.0.1 to independently verify:
- pixel SHA-256 for eight lossless cases, including 8/16-bit, RGB, RLE,
  uncompressed multi-frame, implicit VR, extensionless and MONOCHROME1;
- lossy JPEG decodes without promising source pixel equality;
- truncated DICOM and unrelated text file are rejected.

\`compare.py\` hashes raw native pixel data from each decoder and compares it
against the source manifest. Windowed display output is separate, and its
accuracy is **not** certified by the raw-pixel hash.

Release/CI caveats:
- The three readers load DICOM files eagerly; DCMfx streaming is NOT tested.
- Each case launches a new process, so reported times are micro-smoke
  observations, not reliable cross-library performance rankings.
- No peak RSS, 100 MB/200-frame stress, DICOMDIR, optical drive, clinical
  qualification, or installed Akula integration.
- No PR or change to main.
