# Akula version and release process

Canonical Windows beta version: aquilum-app/native/Cargo.toml, [package].version.
The core Cargo package, both aquilum-core/aquilum-native lock entries and
native/packaging/akula-windows.json must have that exact same three-part version.

## After a signed Windows release

Raise the next development version in a small PR:

    node scripts/akula-version.cjs check
    node scripts/akula-version.cjs next-patch
    node scripts/akula-version.cjs bump 0.3.12
    node scripts/akula-version.cjs check

Bump validates every target in memory before writing; equal, lower or ambiguous
versions are rejected. Required Application CI checks all four files.
No PR merge or main push can publish an Akula release automatically.

The Linux AppImage packager currently stays at its last experimental version
until the independent Linux release approval. check-linux additionally verifies
Linux packaging against the canonical version and deliberately fails while
the Linux channel is on HOLD. Inherited Aquilum package metadata is not the
Akula Windows release authority.

## Signed Windows publication

1. Merge reviewed changes to main and require successful exact-head CI.
2. Launch the existing protected manual workflow
   akula-windows-signed-candidate.yml on main with release_version and
   publish_beta=true. Its independent signatures, identity checks and
   akula-release approval stay mandatory. VERIFY must never publish.
3. Refuse existing tags and any version not above the latest published release.
   Generate the change list from merged PRs since the last published tag.
4. Publish exactly four signed release files: Akula NSIS EXE, its .sig,
   latest.json and latest.json.sig. The tag and signed SHA remain immutable.
5. Verify the installed previous Windows release updates to the new version
   and preserves notes, settings and the separate upstream Aquilum identity.
   Windows beta has no automatic NSIS crash rollback.
6. Immediately bump the next development version in main via review.

Only the public GitHub Release marked Latest feeds the Windows updater.
CI-only candidates and later main commits are not published software.
Linux/macOS signed update publication remains separately blocked.
