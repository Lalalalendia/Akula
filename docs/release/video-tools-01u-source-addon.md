# VIDEO-TOOLS-01u — pinned PyInstaller bootloader and EJS source ADD-ON

**NONRELEASE / NOT COMPLETE. VIDEO-18 RELEASE HOLD.**

## Why this stage exists

VIDEO-TOOLS-01t already creates four exact Git snapshots, including the
custom MinGW PyInstaller *build recipe*; it did **not** include PyInstaller
v6.22.0 source or the yt-dlp-ejs 0.8.0 TypeScript solver sources.
Do **not** recreate or replace that already-accepted four-archive starter.

This stage adds two **supplementary source-only** archives in the existing
PR-only VIDEO-18b-09 evidence workflow. Each file is created using a
verified immutable Git checkout and Git archive with deterministic gzip
settings, not from current upstream branches or floating package versions.

| Add-on source | Immutable identity | What is included / what remains missing |
| --- | --- | --- |
| PyInstaller v6.22.0 | pyinstaller/pyinstaller@70fc17210920bce17f4ab09bbf8104b0dbd45338 | Bootloader C source and waf scripts, PyInstaller Python source modules, package-build recipe, README and COPYING. **Precompiled** tracked PyInstaller/bootloader/Windows-64bit-intel EXEs are explicitly excluded from this source-only archive. The custom MinGW wheel is not included and **not compared byte-for-byte** with this source tree. |
| yt-dlp-ejs 0.8.0 | yt-dlp/ejs@4fb477f4af56880cfd324c48bd4294a2d2294e50 | Complete Git-tracked EJS source, TypeScript solver, Rollup build, hatch_build.py, package.json, npm/pnpm/deno locks and LICENSE. The published sdist and Python wheel bytes are **not** uploaded and the embedded JS has not been rebuilt or source-matched. |

The CI retains the add-on for **one day**, as
video-tools-01u-source-addon-NOT-COMPLETE. The original
video-tools-01t-source-starter-NOT-COMPLETE remains separately available
during its own one-day retention window; the 01u manifest includes SHA-256
digests of those four starter archives for same-PR-HEAD cross-reference.

Neither evidence artifact is an approved distribution package, release,
installer, SBOM or legal attestation. They contain only Git-tracked sources
and JSON/README markers, no Windows EXE, DLL, wheel, signing key, or player data.

## Exact licensing distinction

- The **PyInstaller source** states **GPL-2.0-or-later** with a **bootloader
  exception** in COPYING.txt and bootloader/wscript. Hooks under
  PyInstaller/hooks/rthooks have separate Apache-2.0 terms. Never treat the
  exception as a blanket approval of yt-dlp's entire frozen EXE, which
  upstream classifies as a **GPLv3+ combined work**.
- The yt-dlp/ejs **source** is Unlicense. The **prebuilt EJS Python wheel**
  declares **Unlicense AND MIT AND ISC** because it bundles **astring 1.9.0**
  (MIT) and **meriyah 6.1.4** (ISC). The same versions are checked in
  package.json and package-lock.json, including npm integrity pins.
- EJS upstream release metadata identifies wheel SHA-256
  79300e5fca7f937a1eeede11f0456862c1b41107ce1d726871e0207424f4bdb4
  and sdist SHA-256
  d5fa1639f63b5c4af8d932495f60689d5370f1a095782c944f7f62a303eb104e.
  These are pinned **as metadata**, not independently downloaded or
  rehashed in this source-only stage.

## P0 remaining before any distribution

1. Confirm exact custom MinGW PyInstaller bootloader **wheel-to-source**
   correspondence by acquiring the pinned wheel and build inputs and checking
   the compiled bootloader against its actual source/patch set.
2. Acquire or verify exact source distributions of meriyah, astring,
   Python 3.14, libcurl/OpenSSL and native DLL/PYD files. Check that the
   published EJS wheel and embedded JavaScript correspond to those sources.
3. Acquire matching sources for other pinned Windows Python wheel
   dependencies, including curl-cffi and pycryptodomex, and assess
   whether Git sources and upstream sdists contain *all* corresponding
   compilation/build inputs.
4. Audit Rust mux AGPL linked dependencies, QuickJS-NG MIT attribution and
   the actual notice-and-source distribution mechanism.
5. Independent owner and legal review of the **GPLv3+ corresponding source**
   and license obligations is required before any runtime activation,
   publisher signing, external download or shipping.

All generated manifests retain release_admitted=false,
legal_approval=false, complete_corresponding_source_available=false,
corresponding_source_rebuilt_and_verified=false,
publisher_signing_authorized=false, and windows_executables_included=false.

**VIDEO-18 RELEASE HOLD**, the ordinary Akula two-EXE updater, the existing
six-file NSIS laboratory, the empty approved publisher key and the
closed installed Video Tools queue are unchanged.
