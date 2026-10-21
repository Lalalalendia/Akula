# RELEASE-TRUST-LAB-01 — Offline package authenticity experiment

Status: **research candidate only**. No publisher key is generated for Akula, and this is not release approval.

The lab generates throwaway Minisign keys in a temporary directory on a disposable CI runner. It signs an exact test JSON manifest and exercises an independent offline verifier. It checks the release identity (Akula, version, 40-hex commit, locked Cargo digest), exactly four platform identifiers, unique safe flat filenames, artifact sizes and SHA-256. The verifier first makes a bounded snapshot of the untrusted manifest, verifies the signature over that exact snapshot, and parses the *same bytes* only afterward. It accepts canonical UTF-8 JSON only, rejecting ambiguous duplicate keys. Files are read via no-follow descriptors in 64 KiB blocks so a symlink or multi-gigabyte allocation cannot become an alternative acceptance path. Filenames must match their platform-specific Akula packaging patterns. Negative tests cover same-length changed bytes, modified signed manifest, wrong key, wrong source/version/lock, wrong product, invalid path, duplicated filename, wrong-platform filenames, symlink substitution, noncanonical JSON and missing or unexpected platform.

Run on Ubuntu with minisign and Node.js installed:

`node --test .github/scripts/verify-release-candidate.test.cjs`

This **does not** establish an Akula publisher trust root: a CI-generated test key cannot authenticate the publisher. The production trust root needs a separately controlled private signing key, pinned verified public key in the shipped Akula binary and an approved trust-root rotation/revocation process. Any installation or update requires independent OS-specific package verification, protection against version rollback, executable-health acknowledgement and a tested recovery transaction. The current AppImage `apply.rs` overwrites in place and has no post-start rollback, so the production updater remains disabled.

The existing `release.yml` HOLD workflow, `PUBLIC_KEY = ""`, `channel_ready() = false`, macOS freeze and disabled destructive Sync stay unchanged. This lab does not publish packages, create GitHub Releases, use project secrets, change the installer or run updater apply.
