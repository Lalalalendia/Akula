# ADR-SYNC-037 — macOS publisher-pinned release installation

**Status:** P0-C4 guarded implementation. **No signing identity, Akula GitHub
Release or production installation is enabled by this change.** The local-only
editor feature remains `--features installed`; protected S3 editor is HOLD.

## Problem

A downloaded `Aquilum.app` passing `codesign --verify --deep --strict`
may have an ad-hoc signature, a developer certificate belonging to another
publisher, or a different bundle identity. HTTPS plus a GitHub release URL
confirms transport/origin selection, **not** independent software publisher
identity. The current Akula fork has not provisioned its own verified
Developer ID Application certificate or chosen an Akula-specific signed
bundle identifier. An updater/installer must not treat any generic
structurally signed app as authentic Akula.

## Strict trust roots in the macOS shell installer

1. Source-owned literal `AKULA_DEVELOPER_TEAM_ID` (10 uppercase letters /
   digits) and `AKULA_BUNDLE_ID` (nonempty code identifier) are **both
   deliberately empty** in `scripts/install.sh`. They are not read from
   environment, downloaded `latest.json`, DMG, command-line flags or user
   appdata. Without explicit reviewed provisioning, any new network
   installation fails before the release API is contacted.
2. Once independently provisioned, the *staged* bundle must satisfy one
   pinned **code-signing requirement** checked with
   `codesign --verify --deep --strict -R`: `anchor apple generic`,
   exact signed bundle identifier, Apple Developer ID issuer extension
   `1.2.840.113635.100.6.2.6`, Developer ID **Application** leaf extension
   `1.2.840.113635.100.6.1.13`, and the exact issuer-owned Team ID in
   `certificate leaf[subject.OU]`. Both code signature and requirement must
   verify; ad-hoc or other-Team signatures fail.
3. macOS `spctl --assess --type execute` must also accept the staged app.
   This checks local Gatekeeper policy; **do not** interpret an arbitrary
   Gatekeeper configuration as a portable independent proof of notarization.
   Certificate/signature pinning is the publisher-authenticity requirement.
4. Only after both checks may the installer promote a staged candidate
   using the P0-C3b shared lock, crash recovery and rollback logic.

Sources for the signing requirement syntax:
- [Apple TN3127, Developer ID designated requirement](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements)
- [Apple DTS, Developer ID code signing requirement](https://developer.apple.com/forums/thread/694904)

The inherited `com.dmitriy.aquilum-app` identifier and Aquilum code
signing identity are **not approved** as Akula release trust roots. The
production team's ID and target bundle identifier must be reviewed together
with an Akula signed bundle before embedding those literal values.

## Crash handling must not depend on the new publisher key

A previously installed app may be backed up inside the unique interrupted
installation stage while `~/Applications/Aquilum.app` is missing. The
P0-C3b **pre-network** recovery logic must remain operable even if the
production signing identity has not yet been provisioned. The new trust gate
is intentionally after recovery and before `curl`. Recovery of an old
version is not approval of that version as a newly downloaded Akula build.

## CI boundary

The production installer is directly run against synthetic macOS utilities
with both trust literals empty: it must refuse **before any API request or
filesystem replacement**. A separate test-only **copy** of this exact
installer source substitutes synthetic literal values for the pinned
publisher and bundle identity; the original script in the repository and
release artifacts are untouched. This test copy runs the real staging and
rollback Bash logic against fake DMG and fake `codesign/spctl`. Positive
mock behavior never proves a genuine Apple signature. Negative tests check
wrong signing requirement and denied security assessment, old app/Markdown
preservation, and recovery without a signing key. The existing separate
macOS Actions job still exercises the actual BSD `lockf`.

## Required work before first Akula public release

- Provision a unique Apple Developer ID Application identity, choose the
  signed Akula bundle identifier and record approved 10-character Team ID
  in reviewed source **without runtime override**.
- Set up signing, notarization and stapling of real Akula app bundles and
  DMGs; validate the *actual signed output* with `codesign`,
  `spctl`, Gatekeeper and a clean macOS user. Do not publish a package
  that was merely ad-hoc signed.
- Cross-check the release manifest's artifact URLs, source SHA and
  separately authenticated update metadata; run real first-install,
  update, rollback, interrupted process and uninstall acceptance.
- Retain local-only `installed` mode; creating a publisher identity does
  **not** activate the `sync-installed-host` filesystem authority or
  enable S3 overwrite/delete, baseline promotion, GC or HOLD clearing.

A malicious same-user process able to edit this shell script remains
outside the scope. This gate does not replace OS-owned host custody nor
protect Markdown files from external mutation.
