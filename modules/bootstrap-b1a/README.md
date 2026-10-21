# Bootstrap B1a — shared offline verifier

This package contains the **unchanged** `server_release_lab.rs` verifier as `src/lib.rs`.
It performs only canonical signed-release verification, digest checking and caller-supplied rollback-floor validation. No shell, network, installer, filesystem mutation, signer provisioning or runtime activation is available.

The eight original B1a integration tests are retained. `core-server-bootstrap-b1a-release.yml` checks exact source parity and runs both the standalone tests and legacy Core tests.

B1b0 now imports this package, rather than compiling a second verifier through `#[path]`. Its eight original staging tests also remain in the existing B1b0 workflow, which now triggers on changes to this B1a source because it depends on it.

Production `aquilum_core::server_release_lab` is intentionally unchanged **until a separate Core-adapter PR with complete dependent-contract coverage**. Release signing keys in tests are public fixtures and never trust anchors for real deployments.

Run: `cargo test --manifest-path modules/bootstrap-b1a/Cargo.toml`
