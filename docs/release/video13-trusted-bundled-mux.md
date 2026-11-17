# VIDEO-13 — build-pinned Akula Rust mux candidate (non-release)

## Problem and decision

VIDEO-11 checks that a mux helper produces a valid v2 receipt and nonzero
movie duration. That receipt proves format compatibility **only**: an
arbitrary executable selected via `AKULA_RUST_MUX_BIN` could print the same
text and run untrusted code. VIDEO-12 proves the native two-track pipeline,
not binary provenance or installed GUI behavior.

VIDEO-13 removes the arbitrary-path production selector:

- The app rejects `AKULA_RUST_MUX_BIN` instead of executing it.
- The separate `video-rust-bundle` Cargo feature is **off by default**.
  Existing normal installed builds retain the legacy downloader and do not
  claim a self-contained one-click toolchain.
- A VIDEO-13 Windows candidate must be compiled with
  `AKULA_BUNDLED_MUX_SHA256=<exact helper SHA256>`. The Rust compiler embeds
  this literal using `option_env!`. The feature refuses fallback if the pin
  is missing or malformed.
- At runtime the candidate locates
  `akula-video-mux.exe` **only next to its own executable**. The mux file
  must be a regular non-reparse file with bounded size and match the embedded
  64-character lowercase SHA-256 exactly. Directory reparse points are
  rejected. There is no configurable mux path or self-authenticating
  manifest.
- The helper is built from the exact checked-out Rust `dash` source with
  its fixed `mp4-track = 0.1.0` dependency; CI injects its hash into the
  native test and installed-feature build, then stages the two binaries
  side by side. The helper does not ship in the normal Akula installer.
- The existing versioned v2 receipt, local-only staging, no-clobber
  publication, and MP4 decoding checks remain in place.

## Acceptance and controls

`.github/workflows/akula-video13-trusted-mux-candidate.yml` does not
contact YouTube, publish a release, build an installer, or upload the two
executables. A Windows job builds the source helper, runs the native
exact-digest and same-directory tests, rejects a helper with one changed
byte, compiles `--features installed,video-rust-bundle` with the pin,
and stages the pair in a disposable test directory. Existing VIDEO-12
synthetic DASH, VIDEO-08 mock, and standard Application checks must also
stay green at the exact PR head.

## Release HOLD — security boundaries not proved

**Hash before `CreateProcess` is not a solution to same-user path
replacement races.** A writable per-user installation directory may let
a same-user adversary replace the helper after verification. The Windows
candidate is not a signed, ACL-protected trusted distribution yet. A
production release needs an approved installation/update design, protected
directory semantics, exact binary provenance and signature chain, and
separate installed-app execution tests. Signing only a hash manifest next
to the helper would not solve its trust root.

yt-dlp 2026.10.26 and QuickJS-NG v0.18.0 still rely on separate explicitly
supplied SHA-pinned Windows executable paths. The normal legacy downloader
still uses user-provided tooling when the bundled feature is absent.
Windows 7 remains unsupported by the pinned current yt-dlp executable;
native player support does not imply downloadable content support.

No installed GUI click → progress → restart → playback gate, installer
rollback, real authorized YouTube extraction, cancellation, or third-party
redistribution/license approval is established by VIDEO-13. Keep VIDEO-03
Draft and do not deploy, publish an installer or enable automatic video
downloads on note open.
