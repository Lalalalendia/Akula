# Bootstrap B1b0 — standalone parity stage

This stage introduces a **test-only, non-shipping** Rust package for the existing
Linux B1b0 private signed-artifact staging laboratory. No production Core API,
feature flag, lockfile, installer, SSH, systemd unit, or File Gate path is changed.

The staging implementation in `src/server_staging_lab.rs` is an exact copy of
`aquilum-app/core/src/server_staging_lab.rs`. CI uses `cmp` to detect divergence.
Its `VerifiedRelease` and `ReleaseError` types come from the original
`aquilum-app/core/src/server_release_lab.rs` via Rust's `#[path]` attribute.
That dependency is **shared source**, not a new signing authority or duplicate
cryptographic implementation. The standalone package is not yet an independent
security boundary.

The original B1b0 integration tests are copied, changing only the package import
paths and removing the Core-only feature gate. On Linux, run:

`cargo test --manifest-path modules/bootstrap-b1b0/Cargo.toml`

## Required later work

1. Accept unchanged algorithm and existing crash, ownership, symlink, journal,
   idempotence, and wrong-approval tests in this separate package.
2. Extract B1a release verification into one separately tested shared crate;
   both Core and B1b0 must use the same `VerifiedRelease` type. Do not copy
   the signing verifier and assume type-equivalence.
3. Only after B1a/Core compatibility passes, replace the old B1b0 Core module
   with an API-compatible re-export and regenerate the main Cargo.lock via Cargo.
4. Then shrink broad PR path subscriptions on this workflow in a distinct PR,
   preserving the required Application checks and explicit security coverage.

The package cannot execute, install, activate, or authorize a Node binary.
