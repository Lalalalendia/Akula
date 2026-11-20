# VIDEO-TOOLS-01y — exact native wheel member bytes vs frozen yt-dlp CArchive (NONRELEASE)

**Status:** strictly diagnostic, no publisher signature, no complete corresponding source, no executable release. **VIDEO-18 RELEASE HOLD.**

## Why this audit matters

VIDEO-TOOLS-01x [PR #1082](https://github.com/Lalalalendia/Akula/pull/1082) acquired six exact pinned PyPI source distributions and confirmed all six sdist SHA-256 digests in the `yt-dlp@2026.10.26` build lock. This **does not** identify the specific Windows wheel binaries frozen into `yt-dlp.exe`, or the provenance of native DLL/PYD files.

This PR narrows that uncertainty by using the **real immutable executable**, but it does not manufacture a false claim of full source/wheel equivalence.

## Verified inputs and scope

- Frozen `yt-dlp.exe@2026.10.26`, SHA-256 `f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d` (23,575,744 bytes), source commit `f264a6cbd1f630894da0f368df00f8aae50140e0`.
- The previously accepted static CArchive inventory contains **154 members and 1,574 PYZ module names**. This task extends it by examining only named native `.dll` and `.pyd` members, with bounds-checked zlib decompression and SHA-256 calculation in memory. The Windows EXE is **never run**.
- Re-parse and verify the three upstream requirements files (23 build-install records) against immutable Git source, including the special custom PyInstaller wheel pin and the five-EXE `RELEASE HOLD` manifest.
- Compare **seven** SHA-locked PyPI wheel candidates compatible with normal CPython 3.14 Windows x64: `brotli 1.2.1`, `cffi 2.1.1`, `curl-cffi 0.17.0`, `mutagen 1.48.1`, `pycryptodomex 3.24.1` and the two candidate wheel variants for `websockets 17.3`.
- Wheel bytes are acquired in disposable memory from exact PyPI release metadata over HTTPS and must match one of the original upstream requirement SHA-256 pins. Wheel ZIP paths, count, member sizes and internal distribution metadata are inspected; `setup.py`/install/import/native binaries are **never executed**.
- For each wheel `.dll`/`.pyd`, report one of:
  - `NATIVE_BYTES_EXACT_IN_EXE` — size and SHA-256 exactly match a native CArchive member;
  - `BASENAME_ONLY_NATIVE_BYTES_DIFFER` — named native member exists, but **bytes differ**;
  - `NO_EXE_NATIVE_BASENAME_OR_SHA_MATCH` — no corresponding named or byte-identical native member in the scanned archive.
- A wheel with no native members (e.g. pure Python) **cannot** establish selection by this binary check. Neither exact native match nor name match proves that the *entire wheel* was installed, that native dependencies were built from the downloaded sdist, or that legal notices are complete.

## Evidence delivery and protections

The PR-only workflow uploads **only** one-day `video-tools-01y-native-overlap-JSON-NOT-APPROVAL`, containing SHA-256, native member paths/sizes and explicit unresolved claims. It does not upload wheel, EXE, DLL, PYD, code bodies, signature material or installed components. Downloaded EXE is deleted from the disposable runner.

A successful workflow proves a *bounded inspection ran*, **not** that it found any exact overlap or that all candidate components are available as complete corresponding source. Synthetic negative tests reject traversal, wrong PyPI package/version, wrong candidate wheel set and a basename-only inference as binary equality.

## Remaining P0 after inspection

1. Determine the actual installed wheel set from original build logs or independent wheel-native byte evidence. In particular, a candidate `websockets` wheel selected from two possible compatible files cannot be assigned solely from PyPI metadata.
2. For `curl-cffi`, establish native upstream source/patch/build-input provenance (curl-impersonate/libcurl, TLS/SSL, `.pyd`, bundled DLL and redistributable components). Do not infer that the PyPI `curl-cffi` sdist alone contains all native corresponding source.
3. Establish the CPython 3.14 Windows binary's source/build identity and other native components from actual frozen EXE. Separate potential System Libraries from material subject to corresponding-source obligations via independent legal review.
4. Review complete GPLv3+ frozen EXE / AGPL mux / MIT QuickJS license and notice delivery, exact versioned source availability and independent owner sign-off.
5. Do not rerun 01w's PyInstaller 0/4 byte comparison merely to seek a green reproducibility marker; the toolchain/PE differences have already been documented.

`release_admitted=false`, `legal_approval=false`, `complete_corresponding_source_available=false`, `corresponding_source_rebuilt_and_verified=false`, `publisher_signing_authorized=false`, `binary_sbom_complete=false`. **VIDEO-18 RELEASE HOLD** remains in place. No user-updater, installed media library, customer download, production host, release or signing settings changed.
