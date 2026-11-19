# VIDEO-TOOLS-01i — deterministic unsigned publisher-review custody (NON-RELEASE)

**Purpose:** prepare real Windows 10/11 three-PE package bytes for an eventual independent publisher signature without pretending that a GitHub Actions test key is a publishing key. Follows merged #1054 and #1056. No package is distributed and no real Video Tools installation is authorized.

## Inputs and authority

The existing protected, separate 01a Windows PR laboratory already builds `akula-video-mux.exe` from `labs/video-lite` on exact Git HEAD and independently pins two upstream binaries:

- `akula-yt-dlp.exe` from yt-dlp `2026.10.26`, exact hash and size from `docs/release/video18b04-five-exe-allowlist.json`
- `akula-quickjs.exe` from quickjs-ng `v0.18.0`, likewise pinned by hash and size

The new `scripts/video-tools-01i-unsigned-custody.cjs` consumes only that exact three-file **runner-temporary** stage; rejects unknown files, links, malformed DOS/PE data, wrong machine (`AMD64/PE32+` required), unexpected source SHA/size, promoted release flags, unsupported source revision and unreviewed provenance.

It builds an exact canonical `schema=1` **unsigned** `manifest.json`, with ordered filenames, actual byte SHA-256, actual sizes, the reviewed PR HEAD commit and SHA-256 of the **actual** `labs/video-lite/Cargo.lock`. It independently produces `custody-review.json` with the pinned upstream origins, versions, license-review flags, machine architecture and actual hashes. The custody report is for human review, **not** a seventh installed package object and not a signed authority.

The output is created only in an isolated GitHub-hosted Windows PR runner temporary directory, with no `manifest.minisig`, no private key and no owner signature. It is checked against the independently computed stage hashes immediately before the real isolated NSIS laboratory runs. The existing package install remains the same four-file disposable fixture; it is **not** a fake "signed production install".

## Acceptance and negative cases

- Real installed-stage upstream SHA/size exactly matches the preexisting pinned allowlist.
- Actual three staged files have bounded `MZ/PE` headers, `Machine=0x8664` and `PE32+`; reject ARM/x86/wrong format.
- Exact manifest schema, ordered three named components, source commit and lock checksum are deterministic on repeat.
- Negative Node tests reject forged allowlist/changed upstream bytes, extra EXE, link alias, invalid header, wrong architecture, wrong commit, invalid lock and release admission promotion.
- Existing Windows 01a/01c/01d/01e/01f/01g protections and independent stock two-EXE installer stay unchanged.

## Still blocked before a Video Tools release

- Owner-controlled **separate** Minisign private key custody and out-of-band **public-key approval**; signing the exact reviewed manifest, not regenerating it on the recipient machine.
- Build/source and third-party embedded-dependency license notices, SBOM and provenance review. **Important:** `source_commit` and `cargo_lock_sha256` only attest the Akula mux build context; they do **not** prove the source of upstream yt-dlp or QuickJS. Their distinct pinned URLs/hashes and license-review state are recorded in `custody-review.json`.
- Publisher-signed, independent six-file protected installer with stable product/uninstaller identity, native verification on Windows 10/11, end-user failure/reinstall cases and UI before download activation.
- Full signed-to-running-process custody; 01h only proved a **harmless** native process handoff.

**Explicit HOLD:** `APPROVED_VIDEO_TOOLS_KEY=""`, `shipping_admission_enabled()=false`, `VIDEO_FIVE_CHANNEL_READY=false` and VIDEO-18 RELEASE HOLD remain. No release/deployment/owner signing is part of 01i.
