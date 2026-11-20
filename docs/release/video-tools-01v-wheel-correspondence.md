# VIDEO-TOOLS-01v — pinned wheel/source byte correspondence (NONRELEASE)

This is **investigative evidence**, not a release or complete corresponding source. It extends 01t (four Git snapshots) and 01u (two Git snapshots) without replacing them. No product runtime, updater, signing, VPS, permanent storage, or release workflow changes.

## Evidence being generated

1. Read published `yt_dlp_ejs-0.8.0-py3-none-any.whl` and sdist, verify SHA-256 against immutable 01u pins and PyPI metadata. Compare nine sdist source/build files to exact Git commit `4fb477f4af56880cfd324c48bd4294a2d2294e50`; rebuild minified JS with upstream-pinned Deno 2.6.3 and frozen lock, then compare two actual JS wheel members byte-for-byte.
2. Read custom `pyinstaller-6.22.0-py3-none-win_amd64.whl` with exact SHA-256 `294099ec...` and inspect its Windows-64bit-intel bootloader PE entries without executing them. Recompile pinned `pyinstaller/pyinstaller@70fc17210920bce17f4ab09bbf8104b0dbd45338` under MSYS2 MinGW64 using upstream `yt-dlp/Pyinstaller-Builds@5eb86b862d61adc3df0f44e6e0b815b7d74fa3f5` build recipe, and record whether byte-for-byte equal.
3. Negative tests reject changed package pins, incorrect wheel metadata, traversal/duplicate ZIP entries and approval flags. Results are SHA-based JSON artifacts held **one day**; no downloaded executable/wheel is published as an artifact.

**Interpretation:** unequal bootloader bytes do *not* prove the wrong sources were used: the compiler environment, link options and timestamps might differ from the historical release. Matching bootloader bytes are narrow evidence only for those PE images, not for all Python sources. An EJS bundle match demonstrates correspondence of two bundled JS members to rebuilt source but not all third-party JS or the contents of the outer `yt-dlp.exe`.


## Verified outcome and merge decision — 2026-11-20

The [exact-HEAD provenance CI #44780038924](https://github.com/Lalalalendia/Akula/actions/runs/44780038924) completed **SUCCESS as an inspection**, not as a byte-equivalent PyInstaller rebuild. On PR HEAD `5917177a2d5dde6caa8861c286c18f70e4b4d1a1`:

- **EJS:** PyPI wheel/sdist SHA-256 verified, nine selected sdist source/build files match Git commit `4fb477f4af56880cfd324c48bd4294a2d2294e50`, both minified JS wheel members are byte-identical to Deno 2.6.3 frozen-lock rebuild (`js_equal=1`). These two JS objects are proven; the outer frozen yt-dlp EXE and other dependencies are not.
- **PyInstaller:** published 1,101,025-byte custom wheel SHA-256 `294099ecb5fdd2a13ae4c29006d4e335b697a63b6a16cf052b90ec2b40bef05a` verified. Four Windows x64 bootloader members were successfully built using the pinned source and the upstream recipe. Comparison outcome **`rebuilt=4 byte_equal=0`**; none are identical to the wheel under the 2026-11 Windows-2025/MSYS2 toolchain (observed GCC 16.2.0). An ordinary green job is *not* a corresponding-source approval.
- The exact source wheel was published at [upstream release 2026.08.19.215425](https://github.com/yt-dlp/Pyinstaller-Builds/releases/tag/2026.08.19.215425), produced by [upstream workflow #32306212512](https://github.com/yt-dlp/Pyinstaller-Builds/actions/runs/32306212512) on 19 August 2026; all build jobs succeeded. GitHub returns HTTP 410 for the historical Windows x64 job logs, so exact historical MSYS2/compiler/linker package versions are not known from available logs.

**Decision:** accept 01v only as a finished **nonrelease diagnostic pipeline recording the negative PyInstaller comparison**, then investigate 0/4 in a separate 01w task. Do **not** label the wheel source-equivalent or reproduce a historical toolchain without evidence. P0 still includes compiler/linker/PE forensic comparison, complete Python/native corresponding sources, notices and independent approval. Keep all release/approval flags false and VIDEO-18 RELEASE HOLD active.

## Blocking gaps

- Full third-party Python source/wheel and native DLL/PYD/CPython 3.14 correspondence.
- Independently acquired corresponding source for `astring 1.9.0`, `meriyah 6.1.4`, curl-cffi, OpenSSL/libcurl and other transitives.
- Verification that the outer published yt-dlp EXE contains the same EJS bytes as the separately published EJS wheel.
- Cargo transitive sources, MIT/AGPL notices, actual distribution method and independent legal/owner approval.
- Historical bootloader toolchain attestation / strict reproducible-build determination.

All result JSON keeps `complete_corresponding_source_available=false`, `legal_approval=false`, `release_admitted=false`, `publisher_signing_authorized=false` and `corresponding_source_rebuilt_and_verified=false` regardless of partial byte matches.

**VIDEO-18 RELEASE HOLD remains in effect**. CI acceptance must be judged on the exact PR HEAD; post-merge acceptance requires separately verified runs.