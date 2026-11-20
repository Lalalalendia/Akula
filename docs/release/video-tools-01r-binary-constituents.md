# VIDEO-TOOLS-01r — inspect actual pinned yt-dlp Windows EXE (NONRELEASE)

**Stage: evidence only. VIDEO-18 RELEASE HOLD.** No release permission,
publisher signature, legal clearance or product downloader enabling is implied.

## Scope and reason

01q confirmed that the yt-dlp.exe PyInstaller Windows release is a **GPLv3+**
combined work, while yt-dlp's own repository source is Unlicense. It only
checked tagged license texts. The next task is to inspect the **real asset**
without executing it. Its exact identity is:

- Upstream tag: yt-dlp/yt-dlp@2026.10.26, source commit
  f264a6cbd1f630894da0f368df00f8aae50140e0.
- Exact yt-dlp.exe release asset: 23,575,744 bytes, SHA-256
  f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d.
- Locally renamed akula-yt-dlp.exe without changing contents or license.

A separate **PR-only** Ubuntu CI obtains those exact bytes from the pinned
release, verifies size and SHA before processing, and reads the PyInstaller
CArchive table and nested Python PYZ module table using bounded,
standard-library-only code. It **does not execute** the Windows EXE,
unpack payload DLLs or run yt-dlp.

Only a one-day JSON file named
video-tools-01r-embedded-names-HOLD.json is retained. It contains
CArchive entry names/types/sizes and embedded PYZ Python module names,
not binary bytes, scripts, license texts or proprietary user data.

This is a **partial constituent-name inventory**, **not a complete SBOM**.
In particular a name does not by itself identify an exact upstream version,
license, linked native library or corresponding source. It also does not prove
what native/DLL dependencies are resolved by Windows when the EXE runs.
The inspection has no authority to mark a component legally approved.

## Next P0 evidence after the archive inventory

| Component | Currently established | Required before shipping |
|---|---|---|
| yt-dlp PyInstaller binary, GPLv3+ | Immutable EXE hash, tagged README and bundled third-party license text; next: top-level/PYZ names | Reconcile every actual embedded component with package/version/source and applicable notice. Establish complete corresponding source for *this* combined binary, including build scripts and needed non-System Library material |
| QuickJS-NG v0.18.0, MIT | Exact release asset SHA and tagged MIT copyright/permission notice | Human review of PE/native constituents, deterministic inclusion of version-specific MIT notice |
| Akula Rust H264/AAC mux, AGPL-3.0-only | Source-controlled crate and Cargo.lock, narrower Windows Cargo license triage | Determine actual linked runtime constituents, needed transitive notices and version-specific source distribution package |

## Proposed legal/source delivery strategy (not implemented or approved)

For an eventual optional **manual** Windows 10/11 download, host each version's
exact binary and a matching, downloadable corresponding-source archive with
clear adjacent instructions and stable availability. Preserve full versioned
license texts and attribution in a detached notices/source package or a
separately reviewed installer layout. The existing **exact six-file NSIS
laboratory** is deliberately not modified and is *not* a shipping license
layout.

GPLv3 §6(d) allows separate network servers for corresponding source only
under the specified equivalent access, directions and availability conditions.
**A GitHub link to a later master branch, a source-license URL, or a bare
promise to ask upstream is not proof those conditions are fulfilled.**
The release owner must establish the precise sources and reliable access,
with independent legal review:
https://www.gnu.org/licenses/gpl-3.0.html
https://www.gnu.org/licenses/gpl-faq.en.html

No source archive has been built, corresponding-source completeness certified,
notice sidecar approved, or publisher key activated. The existing pinned
QuickJS MIT and Rust AGPL evidence remain review tasks.

**No Windows EXE is uploaded to CI artifacts.** The stock two-EXE updater,
APPROVED_VIDEO_TOOLS_KEY="", VIDEO_FIVE_CHANNEL_READY=false,
release_admitted=false, and **VIDEO-18 RELEASE HOLD** remain unchanged.
