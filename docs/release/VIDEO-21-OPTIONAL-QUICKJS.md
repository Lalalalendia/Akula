# VIDEO-21 — Optional QuickJS-NG for installed Windows Akula

## Decision
Akula v0.3.21 continues to package only Akula and Shiva. The user explicitly
installs `yt-dlp` and **separately** installs QuickJS-NG from
**Settings → System**; neither executable is bundled, fetched at startup,
updated in the background, or included in synchronized notes.

Upstream yt-dlp supports `--no-js-runtimes --js-runtimes quickjs:<absolute-path>`.
The official pinned yt-dlp executable includes the corresponding EJS solver
scripts. The app passes `--no-remote-components`, `--ignore-config`, and
`--no-plugin-dirs` to restrict external executables and configuration.

## Exact official QuickJS pin
- Upstream: `quickjs-ng/quickjs`, Windows x64, v0.18.0
- URL: `https://github.com/quickjs-ng/quickjs/releases/download/v0.18.0/qjs-windows-x86_64.exe`
- Size: `1,873,152` bytes
- SHA-256: `ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7`
- Local target: `%LOCALAPPDATA%/Akula/tools/quickjs/qjs.exe`;
  explicit `--data-dir` test profiles use their own sibling `tools/quickjs`.
- No arbitrary `PATH` runtime, shell invocation, user-supplied executable,
  or automatic fallback to Deno/Node/Bun in the installed download queue.

After the explicit Yes/No confirmation, the background worker downloads into
a unique temporary file, bounds the transfer, checks SHA-256 and exact size,
flushes data, and publishes via same-volume no-clobber hard link. Parent
symlinks/reparse points and non-regular executables are refused. Invalid files
must be removed explicitly before reinstall. The queue verifies **both**
`yt-dlp` and QuickJS before creating video staging or launching a process.

## Scope and limits
- The previous VIDEO-18 bundled, signed three-tool release HOLD remains intact.
- QuickJS-NG's runtime implementation is a third-party executable. A hash pin
  authenticates byte identity to the selected release, not its legal or
  independent security approval; relevant MIT license/notice duties remain.
- QuickJS may use temporary script files when solving YouTube EJS challenges;
  sandboxing and time-of-check/time-of-use risks require explicit product
  review. No automatic remote EJS components are allowed.
- This change does **not** prove real YouTube extraction or decoded playback.
  Actual YouTube E2E and the signed v0.3.21 NSIS upgrade/rollback checks remain
  mandatory release gates.

## Required acceptance
1. Stock installer contains no QuickJS or yt-dlp, and the Windows QuickJS
   status is absent on a fresh profile.
2. Real Windows GUI cancellation creates no executable; explicit confirmation
   fetches pinned official bytes; removal preserves saved video and Markdown.
3. Installed yt-dlp command contains explicit `--no-js-runtimes` followed
   by the exact `quickjs:<absolute path>` and still refuses remote components.
4. Tampered/absent runtime cannot be used and does not touch the local video
   library; CI tests remain isolated and never use personal profiles.
5. All PR CI checks and actual Windows runner acceptance succeed before merge.
   Protected signed candidate uses `release_version=0.3.21`,
   `publish_beta=false` only after real YouTube E2E.
