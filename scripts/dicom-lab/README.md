# Akula DICOM isolated research lab

This branch is a **research harness**, not product DICOM support or a clinical viewer.

Run the dedicated [workflow](../../.github/workflows/dicom-isolated-lab.yml)
on \`lab/dicom-rust-compare-20261026\`. The workflow deliberately is not
attached to the main app Cargo workspace or the default branch's PR gates.

Three independent standalone Rust packages:
- DICOM-rs source 0.10.0 at pinned merge commit bcc6907c62a99a0a53523b5e80a22002ad12a63d (MIT OR Apache-2.0), including #844 RLE regression fix. The published 0.10.0 had an 8-bit monochrome RLE shift; see baseline run #40357312297 (7/8 hashes).
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

## Stage 2: large-file and DICOMDIR evidence

stress_generate.py creates a 100 MiB, 200-frame, 16-bit signed synthetic DICOM.
stress_compare.py runs each of the three readers four times (warmup + 3
repeats), captures process wall time, max resident set size via GNU time,
frame count and raw SHA-256 parity. Results are in stress_results.csv.
This measures eager full-volume decode, **not** random slice access/streaming.
Microbenchmark times are not portable between runner types.

media_probe.py generates a genuine DICOMDIR with pydicom FileSet, verifies
three extensionless image filenames and their relationships, opens a copy
from a different directory, and detects an intentionally deleted reference.
This is a Python-produced synthetic File-set, not a test of Akula Rust
DICOMDIR parsing. No patient data or real optical media involved.

## Stage 3: Rust DICOMDIR reference scanner

The Rust dicomdir_probe target reads the directory dataset using the pinned
DICOM-rs parser, inspects IMAGE records, validates each relative File ID
component, guards against directory traversal / external symlinks, and checks
that three referenced files exist. It also confirms that removing one image
from a copied media directory is detectable. It does **not** yet follow
the directory record pointer offsets to reconstruct the full patient/study/
series tree; that remains an integration requirement.

stress_reference.py validates the 100 MiB uncompressed signed multi-frame
pixel array with an independent pydicom decoder before the Rust timing tests.
