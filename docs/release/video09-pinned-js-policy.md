# VIDEO-09 — pinned QuickJS / yt-dlp execution gate (Windows experiment)

The `VIDEO-08` FFmpeg-free download route must not rely on whatever JS
runtime or yt-dlp executable happens to exist in PATH. YouTube extraction
can execute external JavaScript: a user-provided path or silent automatic
component download is not an acceptable release trust boundary.

## Opt-in rule

The new route is selected **only** if `AKULA_RUST_MUX_BIN` is explicitly
configured. In that case, `AKULA_QUICKJS_BIN` is also required. Before
staging is created, Akula verifies:

- `AKULA_YT_DLP_BIN` resolves to an **absolute** regular local executable;
  version pin: official Windows `yt-dlp.exe` 2026.10.26,
  expected size 23,575,744 bytes and SHA-256
  `f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d`.
- `AKULA_QUICKJS_BIN` resolves to an **absolute** regular local executable;
  version pin: QuickJS-NG `v0.18.0` Windows x86_64,
  size 1,873,152 bytes and SHA-256
  `ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7`.
- Both must be normal files, not symlinks or Windows reparse points.
  Hash verification reads each file in bounded chunks.
- yt-dlp always receives
  `--ignore-config --no-js-runtimes --js-runtimes quickjs:<verified-path>`.
  This disables the default Deno option and constrains selection to one
  known QuickJS runtime. Remote EJS component downloads remain disabled
  by the default (the official yt-dlp executable ships the EJS package).
- On Linux/macOS, the opt-in route rejects execution until separate
  platform-specific builds and integrity pins have been validated.

Without `AKULA_RUST_MUX_BIN`, this change leaves VIDEO-03's existing
download behavior unchanged. No JS or yt-dlp binary is downloaded by the
Akula application. The Windows CI downloads only the two exact assets into
runner-temporary directories, verifies SHA256, and removes them with the
ephemeral runner.

## Deliberate limits

- This is not a release signing system. The Rust muxer itself remains a
  separately specified absolute path, without a pinned distributable.
  A future installer must distribute verified binaries from a
  write-protected trusted application directory, not an arbitrary
  user-writable folder.
- Executable path races, enterprise proxy handling, Windows 7
  compatibility, EJS threat modelling, live authorized YouTube challenge
  responses, and downloader recoverability still need release acceptance.
- `--version` tests only argument parsing, not whether the present-day
  YouTube challenge is solved. Never interpret offline CI as that proof.
- No real media requests, public installer or synchronized media bytes
  are introduced by this PR.
