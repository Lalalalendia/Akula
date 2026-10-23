# RELEASE-SIGNED-CANDIDATE-01 — typed candidate admission (inactive)

> **Historical scope / Windows status (2026-10-22):** This document
> describes a pre-release research stage and is NOT the current Windows
> updater gate. Akula v0.3.9 and v0.3.10 are already independently signed
> and published; the installed Windows release channel now uses the pinned
> Akula key and protected manual NSIS workflow. See
> [AKULA-VERSIONING-01](./AKULA-VERSIONING-01.md) for the active contract.
> The inherited Aquilum release workflow remains HOLD; Linux/macOS
> publisher approval and automatic NSIS crash rollback remain out of scope.

Status: **Rust module implemented; required full CI and real native Minisign test awaiting acceptance; inactive.** No Akula publisher public key has been provisioned; `release::channel_ready()` remains `false`. No installation, autoupdate or publishing is enabled.

Source: [AKULA-MASTER-INDEX-01](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b). Earlier [#471](https://github.com/Lalalalendia/Akula/pull/471) proved disposable Minisign signing and verification of the signed manifest with four-platform hashes.

## Problem

The old update transport consumes an **untrusted** `latest.json` with version, platform URL and asset signature, and only cryptographically verifies bytes later. A signature on package bytes alone does not prove that the untrusted version, platform, URL or downgrade decision is authorized by the publisher. Also, a plain `Vec<u8>` in the apply boundary can lose evidence that these checks actually happened.

## This increment

`aquilum-app/native/src/updater/release/signed_candidate.rs` introduces a private, non-constructible-from-the-outside `SignedCandidate`. A signed manifest is verified **before parsing** with `release::verify` and the **pinned** `release::PUBLIC_KEY`. The schema binds product `Akula`, version, exact Git checkout 40-hex SHA, `Cargo.lock` SHA-256, exactly four platform entries, and for `linux-x86_64-appimage` the canonical Akula filename, positive bounded byte size and SHA-256 digest. Duplicate/unknown fields are denied via typed `serde(deny_unknown_fields)` structs. Stable three-component numeric versions are compared strictly to the installed binary's version: equal and lower versions are rejected; prerelease identifiers and ambiguous versions are refused.

The admission entrypoint `admit()` accepts no caller-controlled trust root or platform. It refuses immediately while the channel is disabled or the pinned Akula publisher key is empty. The internal verifier does not make a candidate executable; this remains a **typed proof preparation layer only**.

## Acceptance

The native Application CI must compile with direct `sha2 = 0.10`, recorded as `sha2 0.10.9` in the v4 `Cargo.lock` because the repository also contains SHA-2 0.11.1. It must pass the typed schema and byte-matching tests, same-size tampering, downgrade, wrong product, wrong platform, filename substitution, malformed entries on the *other* three platforms, unknown/duplicate JSON keys and missing publisher key. The existing required Rust runner also generates an ephemeral CLI Minisign keypair, signs an exact synthetic JSON manifest and executes the ignored-by-default Rust interoperability test with explicit output assertions. A successful cryptographic test must be observed on the exact PR HEAD; merely adding the test code is not acceptance.

### Explicit remaining blockers

- This increment does **not** call `admit()` from `find()` or `Updater::download()`: legacy metadata is not yet a signed manifest. It also does **not** connect `SignedCandidate` to the Linux filesystem transaction; therefore nothing is newly installable.
- Positive native Rust Minisign interoperability test and CI fixture have been added, but they remain **unverified until CI passes**. The disposable CI key is never an actual Akula publisher key. The earlier Node/CLI lab #471 remains a separate independent proof.
- The durable monotonic version-floor witness is not implemented; comparing against the running version prevents only some downgrades, not rollback of the *persisted update policy*.
- Real detached signatures, isolated genuine candidates from a single pinned source commit, publisher key ceremony and independently trusted pinned Akula root are required.
- Generation-bound Linux first-presented GUI ACK, robust crash rollback, and physical OS acceptance remain pending.

Never use Aquilum keys, enable `channel_ready()`, remove old backup, publish releases, modify Aquilum data, unfreeze macOS, or enable destructive Sync in this PR.
