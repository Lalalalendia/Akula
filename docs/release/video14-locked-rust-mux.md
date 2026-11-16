# VIDEO-14 — lock Rust video mux dependency resolution (non-release)

## Why this exists

VIDEO-13 compiles the pure-Rust DASH muxer from project source and embeds
the built helper's SHA-256 into the experimental Akula executable. But each
Windows CI run previously performed `cargo generate-lockfile` immediately
before compiling, which re-resolved compatible transitive dependency versions.
An exact binary hash for a single CI build **does not** make its source-to-
binary dependency graph stable across future runs.

## Change

- `labs/video-lite/Cargo.lock` is now checked in. It was generated in the
  ephemeral, GitHub-hosted Ubuntu runner by the non-release VIDEO-14 bootstrap
  workflow run 43861435127, from the pinned `mp4 = 0.14.0` and
  `mp4-track = 0.1.0` manifest. The committed file has 350 lines, 8,809
  bytes and 38 dependency package records.
- The temporary bootstrap workflow was deleted once the full file was
  reviewed and committed. No long-lived workflow prints resolved dependency
  metadata.
- VIDEO-05, VIDEO-06, VIDEO-12 and VIDEO-13 CI no longer run
  `cargo generate-lockfile`. They check for the reviewed lockfile and
  compile/test the existing same source with `--locked`.
- VIDEO-06, VIDEO-12 and VIDEO-13 subscribe to lockfile changes; VIDEO-05
  already subscribes to `labs/video-lite/**`. VIDEO-13's static gate also
  refuses a missing lockfile.

## What this proves (and does not)

`Cargo.lock` controls Cargo package versions and registry checksums. It
does **not** prove bit-for-bit reproducibility of the Windows EXE, fix Rust's
toolchain version (the workflows still install `stable`), authenticate
crates.io or GitHub Actions as a trust root, or replace code signing. VIDEO-13
still validates the actual same-build helper hash in the app and keeps the
experimental `video-rust-bundle` feature disabled by default.

Before a signed installer, freeze the exact Rust toolchain and build image,
review third-party crate licenses, attest source inputs and installer outputs,
and close the executable path replacement race with an approved protected
installation/update design. Do not ship helper bytes, enable unattended
video downloads or claim Windows 7 compatibility based on this PR.

## Evidence and promotion gate

Require the exact-head VIDEO-05/06/12/13 build and Windows decode checks,
then a separate Application checks pass on VIDEO-03 after merge. Do not merge
this stacked PR or change the parent/main branch on pending or failed checks.
