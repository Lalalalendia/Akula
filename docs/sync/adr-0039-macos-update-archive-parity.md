# ADR-SYNC-039 — Byte-content parity of the macOS DMG and updater TAR

**Status:** P0-C6a staged, read-only inspection only. No update channel enabled.

## Problem

The release job builds a notarizable DMG and a separate `Aquilum_*.app.tar.gz`
archive for the built-in updater. Before enabling the updater, accepting a
valid DMG **must not** implicitly bless a different archive. Even if both
artifacts came from the same source SHA, they are separately packaged files
and must be compared after publication packaging.

## Inspection contract

`scripts/verify_mac_tar_parity.py` accepts the already-mounted
`Aquilum.app` from a verified DMG and its matching platform
`.app.tar.gz`. It never extracts archive members, writes to the mount
or executes the contents.

For each regular file it compares SHA-256, length and mode bits. For each
directory it compares its path and mode; for each symbolic link it compares
the exact relative target, requiring the target to stay lexically inside
the bundle. It rejects absolute/traversal paths, extra archive roots,
hardlinks/device nodes, case/Unicode-colliding paths, duplicate records,
privileged permission bits, unbounded entries and files, and missing
`Contents/Info.plist` or the native app binary. The entire contents
manifest must match.

Limits for this fail-closed preflight: at most 40,000 entries, 256 MiB
per regular file, 1 GiB total file payload, 4,096 UTF-8 bytes per pathname.
The TAR is processed in streaming mode without `extractall`, preventing
a malicious member from writing outside its own package while under review.

This comparison covers ordinary file bytes, permissions and link targets.
It is **not** a bitwise comparison of DMG and TAR, a proof of extended
attributes/resource forks, or a substitute for Apple code signing,
notarization or Akula's future pinned minisign release key. The produced
`.app.tar.gz.sig` still requires separate verification in the publisher
manifest with a reviewed Akula public key before release. That key is
currently unprovisioned and the updater is disabled.

## CI / rollout decision

13 local Linux Python tests exercise positive parity, changed/extra files,
path traversal, symlink escape, case-insensitive name collisions, duplicate
TAR entries, hardlink rejection and size limits. The required Application
PR gate runs these same tests in CI.

P0-C5 is now merged and P0-C6 wires this comparator directly into
its mounted, publisher-pinned DMG verifier, requiring the matching
architecture-specific `.app.tar.gz` BEFORE any package upload.
The release contract rejects omitting the archive argument. Static checks,
synthetic bundles and mock notarization still do **not** count as acceptance
of a genuinely signed Apple DMG or of a real update installation.

Separately, the existing Rust updater TAR unpacker in
`aquilum-app/native/src/updater/tar.rs` is not production-safe:
its unbounded allocations, symlink-target handling and truncated-header
acceptance require a distinct bounded extraction/security PR before the
`channel_ready()` gate may ever be enabled. Re-check end-to-end
SIGKILL/upgrade rollback and same-user writer boundaries.

No `sync-installed-host` release activation, automatic S3 writes,
baseline advancement, GC or HOLD clearing is authorized by this ADR.
