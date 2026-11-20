# VIDEO-TOOLS-01t — exact Git source starter for all three optional executables

**Status: NONRELEASE / NOT COMPLETE. VIDEO-18 RELEASE HOLD.**

## What is now available

The actual Windows `yt-dlp.exe@2026.10.26` was previously checked against SHA-256
`f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d`.
It is a combined **GPLv3+** PyInstaller executable even though its own
repository's source files are under Unlicense. The 01r/01s CI identified
1,574 Python module names and verified that the 45 sections of the embedded
third-party notice match their tagged upstream text after LF/CRLF normalization.

This task creates a **source STARTER**, deliberately **not complete corresponding source**.
The existing [VIDEO-18b-09 PR-only license workflow](../../.github/workflows/akula-video18b09-windows-license-triage.yml)
already checks out the two exact upstream source commits. It now creates three
separate, deterministic gzip-compressed Git archives:

| Snapshot | Exact Git identity | Included |
| --- | --- | --- |
| yt-dlp | `f264a6cbd1f630894da0f368df00f8aae50140e0` | Complete **Git-tracked tree** at 2026.10.26, including build workflow, `bundle/pyinstaller.py`, versioned hash-constrained requirements, `pyproject.toml`, `uv.lock`, source LICENSE and `THIRD_PARTY_LICENSES.txt` |
| QuickJS-NG | `d7215b0ad3fc2c562e8317fd9bca058fd4863804` | Complete **Git-tracked tree** at v0.18.0, including MIT copyright and permission notice |
| Akula Rust mux | Exact checked PR HEAD | Git-tracked `labs/video-lite` tree and root AGPL license; Cargo.lock and actual `src/bin/dash.rs` included |

The tarballs are generated from `git archive HEAD`, with stable prefixes,
ordered Git content and deterministic gzip timestamps. The CI checks the exact
source commits, critical file Git blob identities, and that no Windows EXE or
DLL was accidentally put into any snapshot. The one-day artifact is named
`video-tools-01t-source-starter-NOT-COMPLETE`. It contains only the three
source tarballs, `README-NOT-COMPLETE.txt` and an evidence manifest.
It is **not** a public binary release, publisher-signed source delivery service,
or complete legal notice bundle.

## Windows x64 build inputs pinned from the *actual upstream workflow*

The tagged upstream job builds on Windows 2025 x64 using Python **3.14**,
PyInstaller **6.22.0**, and installed dependencies from the hash-constrained
`bundle/requirements/pip.txt`,
`bundle/requirements/win-x64-pyinstaller.txt` and
`bundle/requirements/curl-cffi.txt`. It executes
`devscripts/update-version.py`,
`devscripts/make_lazy_extractors.py`,
and `python -m bundle.pyinstaller`.

The manifest inventories each pinned build-install package name, version and
number of accepted SHA-256 distribution hashes. Confirmed pinned versions
include **curl-cffi 0.17.0**, **websockets 17.3**,
**PyCryptodomeX 3.24.1**, **mutagen 1.48.1**, and
**yt-dlp-ejs 0.8.0**. These are *upstream build-environment pins*, not
proof that every package is embedded in the exact EXE, and not an acquired
copy of the corresponding dependency source.

The original executable has an embedded `curl_cffi-0.17.0.dist-info` and
`websockets-17.3.dist-info`, independently matching two pins. That is useful
corroboration, but does **not** reconstruct its entire Python environment.

## Critical remaining obligations (P0 before distribution)

1. Acquire and verify source distributions corresponding to the **exact
   installed wheels** in the Windows build, not merely hashes of wheels.
   The current starter has **no dependency source archives**.
2. Establish source identity of the embedded **Python 3.14 interpreter**,
   PyInstaller bootloader, `curl-cffi` native components, OpenSSL/libcurl,
   all native DLL/PYD files and compilation inputs.
3. Reconstruct **yt-dlp-ejs 0.8.0** generator/build chain for the real packed
   JavaScript solvers. The upstream yt-dlp Git tree alone is not enough.
4. Independently establish whether the exact EXE is build-reproducible from
   these sources and the pinned toolchain. No EXE rebuild or equivalence
   attestation occurs in this task.
5. Acquire the Rust Cargo transitive sources and audit actual linked binaries
   and notices for the **AGPL** mux; QuickJS MIT notice and native provenance
   need their own independent approval.
6. Independently review **GPLv3+** corresponding-source obligations,
   method of delivery and retention/access conditions, and obtain explicit
   owner and legal approval before enabling a release. Merely preparing a
   source starter does **not** satisfy those requirements.

The manifest explicitly keeps
`complete_corresponding_source_available=false`,
`corresponding_source_rebuilt_and_verified=false`,
`legal_approval=false`,
`publisher_signing_authorized=false`, and `release_admitted=false`.

**Do not rename these source tarballs to an approved distribution kit.**
The lab's six-file NSIS path, standard Akula two-EXE updater,
`APPROVED_VIDEO_TOOLS_KEY=""`, `VIDEO_FIVE_CHANNEL_READY=false`,
and **VIDEO-18 RELEASE HOLD** are untouched.
