# VIDEO-10 — real yt-dlp + QuickJS + Rust mux, offline DASH lab

**Purpose.** Previous VIDEO-08/09 Windows acceptance used fake yt-dlp and mux
processes. VIDEO-05/06/07 separately tested real toolchain pieces. This lab
tests **real official Windows yt-dlp 2026.10.26**, **QuickJS-NG v0.18.0**,
and our **pure-Rust `dash mux-pair`**, in one pipeline without contacting
YouTube or downloading any externally hosted video.

## Procedure

1. Ubuntu CI generates ~6 seconds of H.264/AAC DASH with separate video/audio
   adaptation sets from the repository's own synthetic 2-second MP4. FFmpeg is
   used **only** for producing this test fixture.
2. Windows downloads the two official executables to a private CI directory,
   checks their exact SHA256 and byte lengths against accepted VIDEO-05 pins,
   builds `dash.exe` from the tested Rust source, then runs a Python
   `ThreadingHTTPServer` bound **only** to 127.0.0.1 on an ephemeral port.
3. The *real* yt-dlp executable examines the local MPD and downloads video-only
   AVC MP4 and audio-only AAC M4A using the **same format filters and hardened
   flags** as Akula VIDEO-09. yt-dlp is told to use only the verified QuickJS,
   not Deno, remote EJS components or user plugins.
4. `dash mux-pair` produces the MP4 **without FFmpeg**. CI uploads the
   synthetic output as a disposable short-lived test artifact.
5. Independent Linux FFprobe identifies codecs and duration and FFmpeg fully
   decodes the combined Windows-generated media. This is a check on Rust output,
   not a runtime FFmpeg dependency.

## Safety and release limits

- The server only accepts connections on localhost, uses synthetic media, and
  does not request youtube.com or any external streaming source.
- No executable download or bundled toolchain is introduced to Akula itself.
- This CI does not test Akula's GUI click, restart or installed native player.
- A passing local MPD test does **not** prove present-day YouTube's EJS
  challenges can be solved. That must be a separate authorized live test.
- `yt-dlp.exe` pinned here requires Windows 10+. Windows 7 download support
  is unresolved. Installation/signature/ACL of all executables remains a
  release blocker; executable hashes by themselves cannot prevent a
  same-user path substitution after verification.
- No automatic downloads or cloud sync of user video files is enabled.

The experiment will not be merged into Akula's actual video feature unless
its exact-head workflow (generation, Windows real toolchain, Linux independent
decode) completes successfully. It is a **non-release lab only**.
