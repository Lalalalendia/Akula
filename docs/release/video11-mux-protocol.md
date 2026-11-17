# VIDEO-11 — fail closed when a stale Rust MP4 muxer is configured

VIDEO-10 (#971) exposed an important gap in VIDEO-08/09: the original Rust
mux helper could exit successfully and produce a nonempty MP4 whose
`elst` timeline forced a **zero movie duration**. The GUI would then
publish the file into the device-local library even though native playback
could not decode usable frames.

The corrected Rust helper already rejects zero-length outputs and adjusts
DASH edit lists before writing MP4. VIDEO-11 adds a narrowly scoped
**application-side compatibility gate**.

- `dash mux-pair` now prints one line with
  `DASH_RUST_MUX_PAIR_PASS protocol=v2 duration_ms=<positive> video_samples=<positive> audio_samples=<positive> ffmpeg_in_runtime=0`.
  It prints that receipt only after writing, validating and flushing the
  MP4. The binary self-checks its nonzero movie duration.
- The Akula worker captures stdout and stderr without blocking the child.
  It refuses publication if the receipt is missing, malformed, unversioned,
  reports zero duration, reports zero samples, or reports FFmpeg use.
- On any failure the local staging directory is deleted; the Markdown
  source URL, library and synchronized vault remain unchanged. The
  existing no-clobber hard-link publish step is reached only after success.
- Tests cover both positive platform mock paths, all invalid receipt
  variants, and a stale helper that exits 0 after writing a file.

The receipt is **not a cryptographic attestation**. A malicious
user-selected executable can emit arbitrary text, and there is no
protection against path-replacement races between SHA verification and
process launch. A production release must compile and sign the known
Rust helper, bundle it in a protected installation directory, remove the
user-supplied executable-path trust boundary, and run installed GUI
click -> download -> playback -> restart tests. Current yt-dlp.exe also
remains Windows 10+; Windows 7 is not silently marked supported.

This PR is stacked on VIDEO-03. No auto-download, FFmpeg dependency,
public release or cloud upload of user video bytes.
