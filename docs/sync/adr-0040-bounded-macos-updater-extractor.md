# ADR-SYNC-040 — Bounded and fail-closed macOS updater TAR extraction

**Status:** implementation under review. This does **not** activate updates or
change the installed-host custody policy.

## Exposure and threat model

The Akula fork currently disables its updater: `channel_ready() == false`
and `PUBLIC_KEY == ""`. No Akula update stream is configured. Nevertheless
the inherited Rust `updater/tar.rs` had three latent hazards:

- It accepted a short/incomplete header as successful end-of-archive.
- It allocated memory from an untrusted TAR-declared size without a cap.
- It created symlinks without validating their destinations, and wrote
  entries via path parents that could themselves be archive-created symlinks.

A future signature-verification bug, compromised update signing key, or a
manually passed payload could turn these into data loss or path traversal.
A valid signature is not sufficient reason for a parser to trust archive
structures.

## New extraction contract

The extractor receives a **fresh empty real staging directory** created by
the caller; an alias or any existing entry causes refusal. It permits only a
single `Aquilum.app/` root with regular files, real directories and relative
symlinks that stay lexically inside this root. Every file's parent directories
are checked without following symlinks; duplicates and case-alias paths are
rejected. Hardlinks, device nodes, privileged modes, sparse override metadata
and unsupported TAR types fail closed.

All headers require valid octal fields and TAR checksums. File bodies and
512-byte padding must be complete. End-of-archive requires two zero blocks
and at most 1 MiB additional zero padding; no truncated header or trailing
payload is silently treated as success. Payload bytes stream to new files
rather than through a size-controlled `Vec`.

Limits: 40,000 headers, 4,096 bytes and 64 components per archive pathname,
16 KiB per TAR metadata extension, 256 MiB per regular file, 1 GiB total
regular-file content, and 64 MiB for the accumulated explicit/case-alias
path-index strings (including parent path prefixes). These are deliberately conservative local-first
update limits, to be checked against real signed release bundles before
activating updates. Essential Info.plist and the native app binary must be
regular files under the signed bundle's expected paths.

The reader supports bounded GNU long-name/link records and checked POSIX
PAX path/linkpath overrides. PAX size and GNU sparse layouts are rejected.
Non-layout PAX metadata is ignored; extended attributes are not restored.

## Acceptance

The 21 standalone Rust unit tests cover valid extraction and an internal
symlink plus truncated headers/body, invalid checksums, malformed endings,
escaped paths and symlinks, writes through a symlink parent, hardlinks,
duplicate/case-colliding entries, oversized file headers, pre-populated
stage directories, missing essential files and post-archive data.

Rust builds on Linux normally **do not compile this macOS-only module**.
Required Application CI therefore includes a standalone `rustc --test`
harness that loads the same production `tar.rs` source, with warnings
treated as errors. Native macOS installer CI now compiles and runs these same Rust tests as well.
An authentic signed `.app.tar.gz` compatibility run is still required before
activating the updater.

## Explicitly not solved

The inherited `updater/apply.rs` still deletes an older staging path,
uses a process-ID-based backup name, replaces the app across multiple
non-transactional renames, and does not share the shell installer's
cross-process lock or reliable interrupted-update recovery. A crash may
leave the user with no app or an unverified generation. That requires
separate P0-C6c refactoring and packaged macOS process-kill acceptance.

No genuine Akula publisher key, notarized DMG, pinned minisign updater key
or complete updater release is provisioned here. This is cooperating
application safety; it does not prevent a malicious arbitrary process with
the same filesystem permissions from altering the staging directory.

The protected S3 writer, remote overwrite/delete, baseline promotion, GC,
and unattended HOLD clearance remain disabled.
