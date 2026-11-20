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

## Actual first-pass result — 20 November 2026

The exact PR-head [native source audit run #44793379710](https://github.com/Lalalalendia/Akula/actions/runs/44793379710) succeeded. It downloaded and hash-verified **seven** normal CPython 3.14 Windows x64 wheel candidates, then statically inspected **119** native DLL/PYD entries in the exact pinned `yt-dlp.exe`. There are **47 byte-for-byte native wheel ↔ EXE archive member overlaps** (size and SHA-256, independently checked in each source): 

| Exact locked distribution | Native wheel file matches | Key evidence |
| --- | ---: | --- |
| brotli 1.2.1 | 1/1 | `_brotli.cp314-win_amd64.pyd` |
| cffi 2.1.1 | 1/1 | `_cffi_backend.cp314-win_amd64.pyd` |
| curl-cffi 0.17.0 | **2/2** | `curl_cffi/_wrapper.pyd`, `curl_cffi.libs/libcurl-impersonate-ae864b926b0a86bfa3b18a664407a1ab.dll` |
| pycryptodomex 3.24.1 | **42/42** | All native `Cryptodome/**.pyd` payloads |
| websockets 17.3 (native cp314 wheel) | 1/1 | `websockets/speedups.cp314-win_amd64.pyd` |
| websockets 17.3 (pure py3 wheel) | 0/0 | No native member to test; the compiled `speedups` comes from a native-compatible payload |
| mutagen 1.48.1 | 0/0 | Pure-Python package, not identifiable by a native-byte check |

The remaining **72** native EXE entries are not covered by these wheel matches. Important unassigned files include `python314.dll`, `python3.dll`, `libssl-3.dll`, `libcrypto-3.dll`, `libffi-8.dll`, `ucrtbase.dll`, `VCRUNTIME140*.dll`, numerous Python standard-library `.pyd` files and Windows API-set DLLs. These are a **review queue**, not an accusation of missing licenses. Whether a library qualifies as a GPLv3 System Library requires a separate fact- and license-specific determination.

This evidence strongly narrows the candidate wheel provenance, especially `curl-cffi`. It **does not** certify that a whole wheel or all transitive libraries came from the published sdist, identify the original pip installation log, prove reproducible native compilation, or authorize the bundled program to ship.

## Remaining P0 after inspection

1. Determine the actual installed wheel set from original build logs or independent wheel-native byte evidence. In particular, a candidate `websockets` wheel selected from two possible compatible files cannot be assigned solely from PyPI metadata.
2. For `curl-cffi`, establish native upstream source/patch/build-input provenance (curl-impersonate/libcurl, TLS/SSL, `.pyd`, bundled DLL and redistributable components). Do not infer that the PyPI `curl-cffi` sdist alone contains all native corresponding source.
3. Establish the CPython 3.14 Windows binary's source/build identity and other native components from actual frozen EXE. Separate potential System Libraries from material subject to corresponding-source obligations via independent legal review.
4. Review complete GPLv3+ frozen EXE / AGPL mux / MIT QuickJS license and notice delivery, exact versioned source availability and independent owner sign-off.
5. Do not rerun 01w's PyInstaller 0/4 byte comparison merely to seek a green reproducibility marker; the toolchain/PE differences have already been documented.

`release_admitted=false`, `legal_approval=false`, `complete_corresponding_source_available=false`, `corresponding_source_rebuilt_and_verified=false`, `publisher_signing_authorized=false`, `binary_sbom_complete=false`. **VIDEO-18 RELEASE HOLD** remains in place. No user-updater, installed media library, customer download, production host, release or signing settings changed.
