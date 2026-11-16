# VIDEO-17 — actual NSIS-installed video stack, non-release acceptance

## Release boundary and purpose

VIDEO-16 proves a real GUI can load local DASH, persist private MP4, close,
reopen and decode a frame using four manually staged executables. It does not
prove **cargo-packager NSIS actually puts those executables on disk**.

The current shipping `aquilum-app/native/packaging/akula-windows.json`
contains only the Akula main EXE and the pinned Shiva PDF companion.
It MUST stay unchanged until a complete signing/update and OS-layout decision.

This PR adds **only** `akula-windows-video17-lab.json`, copied from that
shipping config with the three pinned media EXEs appended as non-main
binaries. Its ordinary Akula product/registry identity means the workflow
runs only on a clean, hosted disposable Windows runner. It must never run on
a user's PC.

## Actual Windows CI proof

1. Confirm the normal shipping config still contains exactly two binaries,
   while the lab config contains the exact five expected executables and
   no other changed configuration fields. The `video-ui-e2e` feature must
   be absent from the default `installed` feature.
2. Generate a six-second synthetic DASH fixture: independent H.264/AAC
   tracks, relative segment URLs and **no live YouTube requests**.
3. Build the reviewed locked Rust DASH mux and pass its SHA-256 into the
   compiled Akula GUI candidate, retrieve exact SHA/size-pinned official
   yt-dlp 2026.10.26 and QuickJS-NG v0.18.0, and separately compile
   `Lalalalendia/shiva@40b72a90b1dac39812c77c7321e57d613a81cb51`.
4. Compile real Windows Akula in the non-release
   `installed,video-rust-bundle,video-ui-e2e` feature set and put five
   exact executables under the existing Cargo release directory.
5. Run pinned `cargo-packager 0.11.8` with **only** the new lab config to
   generate a real NSIS installer. Do not sign it, upload it or publish.
6. On the same clean Windows runner, install the actual generated NSIS
   with `/S`. Require Akula's own HKCU uninstall identity, a regular
   file for every expected EXE and bit-identical SHA-256 vs the pre-NSIS
   stage. Fail if any companion is missing or replaced.
7. Run VIDEO-16's unchanged real Windows pointer-input test **against the
   NSIS-installed EXE** in `%LOCALAPPDATA%\Akula`, with an isolated
   temporary vault/profile and localhost-only DASH. Require a real
   Download click, private MP4, unchanged note, clean app restart,
   real Play click and decoded GUI frame.
8. Uninstall the same NSIS package, require exact Akula executable and
   registry removal, intact Markdown and no Aquilum identity modification.
9. Upload **only** the short synthetic MP4 for an independent Ubuntu FFmpeg
   H.264/AAC decode and frame-count check. No installer, signed metadata,
   private data or executable bundle is published.

## Security caveat — this is not protected distribution

The legacy per-user Akula installation in `%LOCALAPPDATA%` is writable by
the same user who runs Akula. A SHA-256 check of a sidecar before
`CreateProcess` does **not** prevent an attacker of that same user
from replacing the file between verification and launch. This lab does
not fix that. It deliberately validates current NSIS behavior without
claiming tamper-proof security.

Before a public self-contained video release, separately decide and prove:

- Protected per-machine install/update and its privilege boundary, **or**
  a reviewed same-user launch architecture that actually eliminates
  hash-to-spawn path substitution. Do not pretend default per-user ACLs
  give an immutable executable.
- The correct owner-signed installer and publisher manifest covering all
  distributed binaries, authenticated update/rollback and independent
  protected-release approval. The existing Akula publisher pipeline remains
  manual, protected and unrelated to this PR.
- Redistribution license/notice inventory for Rust crates, yt-dlp, QuickJS
  and Shiva. The current upstream official yt-dlp binary is not proof of
  Windows 7 support. Test actual platform support separately.
- A versioned upgrade, rehash and rollback test for the entire five-file
  installation, including a failed partial update.

**Do not merge this lab onto `main` or enable the test feature in public
packaging.** Accept only after exact-commit Windows NSIS + real GUI and
independent decode pass, then a full Application CI on parent VIDEO-03.
