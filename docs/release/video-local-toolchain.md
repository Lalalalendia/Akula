# Akula VIDEO-04 — offline video toolchain / release gates

Status: **research and CI experiment only**. This file and the Windows lab are
not authorization to package, install, or auto-update any third-party executable.

Related implementation: VIDEO-01 PR #961 (merged), VIDEO-02 PR #962 (merged),
VIDEO-03 PR #963 (native preview/download/playback, Draft on 2026-11-16).

## Invariants

1. Inserting/opening a YouTube URL never starts yt-dlp or fetches the video.
   Only the explicit **Download** action may enqueue a local download.
2. A synced Markdown note contains only the URL/text. An absolute device path,
   MP4/WebM, partial file, or yt-dlp staging directory **must not** enter a
   vault, S3, Dropbox, Yandex Disk, or any other sync payload.
3. Videos live in the per-device media library; two notes may reference one
   local copy, and a second device needs a separate explicit download.
4. Never execute a fetched downloader before checking its pinned checksum.
   No silent updater, auto-installer, or remote EJS component fetching.
5. Only download content when the user is authorized and source terms permit it.

## OS and feature support are distinct

| OS | Akula native H.264/AAC playback | Official 2026.10.26 yt-dlp.exe | VIDEO-04 release readiness |
| --- | --- | --- | --- |
| Windows 10/11 | Native Windows Media Foundation; offline fixture CI | Official Win10+ binary | **Not ready**: FFmpeg, JS runtime, packaging, GUI still unproven |
| Windows 8/8.1 | Native path may work, needs real OS validation | Current official binary is not supported | Not ready |
| Windows 7 SP1 | Basic MF RGB32 fallback in VIDEO-03; **real VM test still required** | Current official binary is not supported | Not ready; keep link preview and local-file playback separate from downloader support |

Do **not** fix Windows 7 by silently bundling an old, unmaintained downloader.
A tested and maintained Win7-specific toolchain is a separate research outcome,
not an assumption. Compiling Akula for a Win7 target or invoking a basic MF
decoder on a modern Windows runner does not prove Win7 end-to-end behavior.

## Exact initial pin for the *isolated Windows 10 lab*

- Upstream project: https://github.com/yt-dlp/yt-dlp
- Release: `2026.10.26` (October 26, 2026)
- Official asset: `https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp.exe`
- SHA-256: `f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d`
- Lab command: `pwsh ./scripts/akula-video-deps-win10-lab.ps1 -ScratchRoot <disposable-directory>`
- It first verifies a tampered file is rejected, downloads the exact asset to
  an isolated temporary directory, verifies SHA-256, executes `--version` only,
  then removes the temporary bytes. No real video, cloud sync, or release upload.
- Version-only success means **nothing** about installed GUI, Win7 support,
  codec merging, ability to download a specific video, or distribution legality.

## Mandatory dependency acceptance before packaging

- [ ] `yt-dlp`: signed/pinned source and digests, license review, version/health
      probe, useful error for missing or incompatible version, secure update
      policy. If an external user-installed version is allowed, document the
      trust boundary and execution risks.
- [ ] `ffmpeg` / `ffprobe`: select a concrete release and hash, check its
      redistribution license and codec support, verify MP4 AVC/AAC merging and
      that no accidental executables are found in a synced workspace.
- [ ] JavaScript runtime: choose a supported pinned Deno/QuickJS/Node build,
      test actual YouTube challenge execution using authorized test material,
      and restrict capabilities. Do not enable `--remote-components`
      implicitly. The official standalone yt-dlp binary includes EJS scripts,
      but **still needs** an external JS runtime.
- [ ] Windows 7 SP1: real workstation VM acceptance, not Windows Server or a
      cross-compiled binary alone. Either verify a maintained compatible
      downloader or clearly disable *video downloading* while keeping notes
      and link previews usable.
- [ ] Installed GUI: click → explicit request → progress → success/failure →
      application restart → local playback, including a missing-tool failure.
      Capture the exact executable SHA and logs.
- [ ] Cross-device: sync only Markdown URL, never video bytes or absolute paths.
- [ ] Local storage: enforce quota, cancellation, temporary-file cleanup,
      removal of local video without modifying notes, and symlink/junction
      exclusions at the sync boundary.
- [ ] Release and legal review: yt-dlp standalone executables include third-party
      components and may entail GPLv3+ obligations, depending on redistribution
      and integration. See THIRD_PARTY_LICENSES.txt and consult qualified review
      before putting these artifacts inside an Akula installer.

## Upstream references (verified 2026-11-16)

- yt-dlp supported Python/OS versions and recommended dependencies:
  https://github.com/yt-dlp/yt-dlp
- Official 2026.10.26 release and asset hashes:
  https://github.com/yt-dlp/yt-dlp/releases/tag/2026.10.26
- EJS scripts and JS runtime requirements:
  https://github.com/yt-dlp/yt-dlp/wiki/EJS
- FFmpeg Windows release constraints (Win7 targeted with UCRT; no guarantee
  on systems below Windows 10 22H2):
  https://github.com/BtbN/FFmpeg-Builds

Final acceptance is based on exact-head workflow evidence and an installed
workstation test, not just source review or a label in the PR.

## VIDEO-04 FFmpeg + Deno pinned remux lab (Windows 10+)

Non-release proof on **synthetic authorized bytes only**. Do not redistribute
any downloaded archive or publish a tested executable automatically.

| Dependency | Immutable reference | SHA-256 |
| --- | --- | --- |
| FFmpeg, LGPL static | \`FFmpeg-Builds/FFmpeg-Builds\`, \`autobuild-2026-11-15-12-40\`, \`ffmpeg-n8.1.3-35-gf1acb0a266-win64-lgpl-8.1.zip\` | \`272140baa96cbb9553eb57c03590e0b8d2c93ce352aa7f876c34fbc657536836\` |
| Deno | \`denoland/deno\`, \`v2.9.8\`, \`deno-x86_64-pc-windows-msvc.zip\` | \`cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf\` |
| yt-dlp | \`yt-dlp/yt-dlp\`, \`2026.10.26\`, \`yt-dlp.exe\` | \`f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d\` |

The lab in \`scripts/akula-video-deps-ffmpeg-deno-lab.ps1\`:
- rejects a tampered archive, then independently checks each downloaded SHA
  before extracting or executing third-party code;
- verifies Deno \`--version\` and basic JS evaluation and checks that yt-dlp
  accepts the chosen Deno runtime argument **without contacting YouTube**;
- runs pinned FFmpeg/ffprobe against a tiny, original H.264/AAC fixture,
  splits video and audio streams, remuxes them with \`-c copy\`, checks codec
  identities, and decodes the result;
- records compressed archive and executable byte counts, and deletes scratch
  files. It does not modify or install into any Akula vault or cloud folder.

**Limitations:**
- FFmpeg LGPL variant does not include the GPL x264 encoder; stream copy of
  already encoded H.264/AAC is the intended initial use. If user-provided
  content needs re-encoding, it is a separate codec + license decision.
- The test does not exercise network-based YouTube challenge solving or
  verify the runtime sandbox against hostile scripts.
- Windows 7/8 support is **not** established for these binary releases.
- Real installer packaging, dependency update policy, codec compatibility
  on actual user machines, and the Windows 7 VM remain release blockers.
- Even an LGPL binary needs attribution and compliance review; Deno and
  yt-dlp each carry their own licenses and bundled third-party components.
- An archive of ~168 MB is not a good default installer payload without
  measuring smaller, reproducible packaging or optional, explicitly installed
  dependencies.

Pin metadata read from GitHub Release API on 2026-11-16, not inferred from
floating \`latest\` links:
https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/tag/autobuild-2026-11-15-12-40
https://github.com/denoland/deno/releases/tag/v2.9.8
https://github.com/yt-dlp/yt-dlp/releases/tag/2026.10.26
