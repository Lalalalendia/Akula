# VIDEO-TOOLS-01x — six priority Python source distributions from pinned yt-dlp build inputs (NONRELEASE)

**Status: investigative SOURCE-ONLY acquisition, NOT COMPLETE. VIDEO-18 RELEASE HOLD.**

## Provenance and narrow scope

Following merged 01t–01w, the source starter already covers six immutable Git source snapshots, and 01v proves **2/2** EJS minified wheel JS bundles match the pinned Deno build. 01w proves all four PyInstaller bootloader EXEs have materially different PE sections when rebuilt with current MSYS2, so their historical byte equivalence is **not** proven. Stop spending CI time on blind re-runs and prioritize missing third-party Python/native source inputs.

The exact upstream `yt-dlp/yt-dlp@f264a6cbd1f630894da0f368df00f8aae50140e0` build uses three SHA-hash-constrained Python requirements files:

| Requirements file | Installed-input *records*, not EXE contents |
| --- | ---: |
| `bundle/requirements/curl-cffi.txt` | 14 |
| `bundle/requirements/win-x64-pyinstaller.txt` | 8 (7 normal PyPI, 1 **custom** PyInstaller wheel) |
| `bundle/requirements/pip.txt` | 1 |
| **Total** | **23** (22 PyPI and 1 custom wheel, before platform-marker filtering) |

This stage deliberately obtains **six** high-priority source distributions from PyPI: `curl-cffi 0.17.0`, `cffi 2.1.1`, `pycryptodomex 3.24.1`, `brotli 1.2.1`, `mutagen 1.48.1` and `websockets 17.3`. Other packages remain an explicit review queue. `yt-dlp-ejs 0.8.0` already has separate verified 01u/01v source evidence and is not downloaded again.

## Checks and retained evidence

- Checkout exact Akula PR HEAD and immutable upstream yt-dlp source commit. Verify all 23 build-install pins, package versions, custom PyInstaller SHA, and the unchanged five-EXE release-HOLD allowlist.
- For the six selected packages, query exact PyPI release metadata, require matching normalized name/version, read only immutable HTTPS `files.pythonhosted.org` sdists, and hash them against the SHA-256 listed by PyPI. Check **independently** if each actual sdist SHA also appears in the original upstream hash-constrained requirements file. An sdist that is merely published by PyPI is identified separately from an upstream-hash-pinned sdist.
- Inspect source tar/ZIP as **inert archive metadata only**: enforce safe paths and size/count bounds, record whether build metadata and C/C++/Rust/Cython source filenames are present. Never execute `setup.py`, install dependencies, extract to arbitrary paths, import the downloaded source, or build a wheel.
- Use PyPI release metadata to list **normal CPython 3.14 Windows x64 compatible** wheel names whose **metadata hashes** appear in the upstream lock; do not mistake cp314t free-threading, PyPy, or other CPython version wheels for installed candidates. Do **not** download executable wheels; a matching version/hash list does not establish which wheel actually went into the frozen EXE.
- Preserve actual downloaded *source archives only*, plus a JSON review manifest and README, as a PR-only one-day GitHub Actions artifact `video-tools-01x-six-pypi-sdists-NOT-COMPLETE`. Nothing is publicly published as approved corresponding source. No EXE, DLL, wheel, installer, signing key or user data is in the artifact.

## First verified acquisition result (PR HEAD `ab834648`, 20 November 2026)

The source-only workflow [run #44788396207](https://github.com/Lalalendia/Akula/actions/runs/44788396207) was evaluated on its exact PR HEAD and completed **SUCCESS**. The PyPI sdist SHA-256 matched an original upstream requirements-file pin for **all 6/6** selected packages; the archive inspection found their build metadata and produced a source-only artifact with no wheels, DLLs or EXEs.

| Pinned distribution | Acquired sdist bytes | Source archive entries | Native-like source-file names |
| --- | ---: | ---: | ---: |
| brotli 1.2.1 | 7,389,447 | 213 | 108 |
| cffi 2.1.1 | 530,807 | 264 | 52 |
| curl-cffi 0.17.0 | 282,294 | 115 | 15 |
| mutagen 1.48.1 | 1,276,978 | 302 | 0 |
| pycryptodomex 3.24.1 | 4,934,627 | 711 | 111 |
| websockets 17.3 | 188,878 | 76 | 1 |

The counts indicate source archive contents only, not exact embedded/native library coverage. Initial source-lock checks succeeded, but the **PR is still awaiting final exact-HEAD CI following a stricter CPython 3.14 Windows x64 wheel-*metadata* candidate filter**. Avoid turning source availability into installed-wheel identity or compliance claims.

## Do not overstate the result

An acquired and SHA-verified PyPI sdist is **not sufficient** to demonstrate source correspondence to the exact Windows wheel, reproducible compilation, completeness of native transitive libraries, or license compliance. `curl-cffi` in particular requires separate examination of native curl/SSL/impersonation libraries and patches. Windows CPython 3.14 embedding, native `.pyd` files, external DLLs, wheel installation selections, and any System Libraries exemption remain open.

The six archives are only a subset of the 23 build-install pins, which themselves are **not** the complete executed/linked-component inventory. Complete source-and-notice delivery must also reconcile Rust AGPL and QuickJS MIT separately. Independent legal/source review and explicit owner approval are still required before any release.

Hard-coded output gates always remain `release_admitted=false`, `legal_approval=false`, `complete_corresponding_source_available=false`, `corresponding_source_rebuilt_and_verified=false`, `publisher_signing_authorized=false`, `binary_sbom_complete=false`, `notices_packaging_approved=false`.

**VIDEO-18 RELEASE HOLD remains active. No runtime, updater, signed installer, VPS, permanent storage or customer-facing downloader is modified.**
