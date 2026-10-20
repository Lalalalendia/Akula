# RELEASE-SIGNED-CANDIDATE-01 — typed candidate admission (inactive)

Status: **Rust module compiled and unit tested, but inactive.** No Akula publisher public key has been provisioned; `release::channel_ready()` remains `false`. No installation, autoupdate or publishing is enabled.

Source: [AKULA-MASTER-INDEX-01](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b). Earlier [#471](https://github.com/Lalalalendia/Akula/pull/471) proved disposable Minisign signing and verification of the signed manifest with four-platform hashes.

## Problem

The old update transport consumes an **untrusted** `latest.json` with version, platform URL and asset signature, and only cryptographically verifies bytes later. A signature on package bytes alone does not prove that the untrusted version, platform, URL or downgrade decision is authorized by the publisher. Also, a plain `Vec<u8>` in the apply boundary can lose evidence that these checks actually happened.

## This increment

`aquilum-app/native/src/updater/release/signed_candidate.rs` introduces a private, non-constructible-from-the-outside `SignedCandidate`. A signed manifest is verified **before parsing** with `release::verify` and the **pinned** `release::PUBLIC_KEY`. The schema binds product `Akula`, version, exact Git checkout 40-hex SHA, `Cargo.lock` SHA-256, exactly four platform entries, and for `linux-x86_64-appimage` the canonical Akula filename, positive bounded byte size and SHA-256 digest. Duplicate/unknown fields are denied via typed `serde(deny_unknown_fields)` structs. Stable three-component numeric versions are compared strictly to the installed binary's version: equal and lower versions are rejected; prerelease identifiers and ambiguous versions are refused.

The admission entrypoint `admit()` accepts no caller-controlled trust root or platform. It refuses immediately while the channel is disabled or the pinned Akula publisher key is empty. The internal verifier does not make a candidate executable; this remains a **typed proof preparation layer only**.

## Acceptance

The native Application CI must compile with `sha2 = 0.10` already present in `Cargo.lock` and pass tests for exact manifest shape and byte matching, same-size byte tampering, downgrade, wrong product, wrong platform, filename substitution, unknown or duplicate fields, and missing publisher key.

### Explicit remaining blockers

- This increment does **not** call `admit()` from `find()` or `Updater::download()`: legacy metadata is not yet a signed manifest. It also does **not** connect `SignedCandidate` to the Linux filesystem transaction; therefore nothing is newly installable.
- Positive Rust integration tests with an independently generated Minisign test key must be added; structural unit tests alone do not prove signing interoperability. The earlier Node/CLI lab #471 is separate.
- The durable monotonic version-floor witness is not implemented; comparing against the running version prevents only some downgrades, not rollback of the *persisted update policy*.
- Real detached signatures, isolated genuine candidates from a single pinned source commit, publisher key ceremony and independently trusted pinned Akula root are required.
- Generation-bound Linux first-presented GUI ACK, robust crash rollback, and physical OS acceptance remain pending.

Never use Aquilum keys, enable `channel_ready()`, remove old backup, publish releases, modify Aquilum data, unfreeze macOS, or enable destructive Sync in this PR.
