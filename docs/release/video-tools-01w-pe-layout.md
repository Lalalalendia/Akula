# VIDEO-TOOLS-01w — static PE layout forensics for PyInstaller x64 (NONRELEASE)

This is a narrow follow-up to accepted [VIDEO-TOOLS-01v PR #1079](https://github.com/Lalalalendia/Akula/pull/1079). **01v confirmed SHA-pinned EJS source-to-wheel minified JS equality (2/2) but a PyInstaller wheel-to-current-Waf rebuild byte mismatch (0/4).** Neither result grants distribution rights or proves complete corresponding source.

## What 01w adds

- Reuse the exact published `pyinstaller-6.22.0-py3-none-win_amd64.whl` SHA-256 `294099ecb5fdd2a13ae4c29006d4e335b697a63b6a16cf052b90ec2b40bef05a`, the source Git commit `70fc17210920bce17f4ab09bbf8104b0dbd45338` and the custom build recipe commit `5eb86b862d61adc3df0f44e6e0b815b7d74fa3f5`.
- Recompile four Windows-64bit-intel bootloader images using the exact Waf command used upstream, in a disposable Windows-2025/MSYS2 job. **Do not execute the downloaded wheel or its PE members.**
- Parse static PE32+ DOS/COFF/optional headers and section-table bounds; record file SHA-256, size, timestamp, checksum, linker-version fields, subsystem and section content SHA-256. Compare `.text`, `.rdata` and all other present sections without copying the code bytes into the report.
- Check whether all changed bytes are limited to the four-byte COFF timestamp and four-byte optional-header checksum. A different `.text` hash is evidence of changed machine-code section bytes, but not by itself evidence of wrong source.
- Record the **current** `gcc`, `ld`, and installed `pacman` GCC/binutils/CRT versions. These do not attest the August 2026 runner.
- Publish only an exact-PR-HEAD one-day nonapproval JSON, never any executable/wheel, signing key or source package falsely designated complete. Fail closed on malformed PE, missing rebuilt files, altered wheel or release-gate drift.

## Historical evidence and limitation

The original custom wheel appeared at [upstream release 2026.08.19.215425](https://github.com/yt-dlp/Pyinstaller-Builds/releases/tag/2026.08.19.215425). Its Windows x64 build job succeeded in [upstream run #32306212512](https://github.com/yt-dlp/Pyinstaller-Builds/actions/runs/32306212512), 19 August 2026. Its historical logs are expired (GitHub HTTP 410), so the exact compiler/linker/CRT packages present on that runner are **not proven**.

The closest MINGW-packages source recipes before that build suggest GCC 16.2.0 package release 3, binutils 2.47 release 3, and mingw-w64 CRT 14.0.0.r283. The 20 November 01v rerun log installed GCC 16.2.0 release 7, binutils 2.47 release 5, and CRT r598. Source recipe revision history **is not** the original runner's installed package manifest.

## Decision boundary

A GREEN 01w job means **valid evidence was collected**, regardless of which sections differ. The result must not be reported as a byte-identical rebuild unless all four hashes actually match. If machine-code section contents differ, investigate the toolchain/flag provenance as a separate question; do not try arbitrary substitutions or disable safety gates. GPL/AGPL corresponding-source obligations, third-party Python/native source packages, notices and legal approval remain separate P0 work.

All generated JSON preserves `complete_corresponding_source_available=false`, `corresponding_source_rebuilt_and_verified=false`, `legal_approval=false`, `release_admitted=false`, `publisher_signing_authorized=false`. **VIDEO-18 RELEASE HOLD stays in effect.**
