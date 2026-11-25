# Bootstrap B1b0 — shared inert staging

This non-shipping Rust package owns the single Linux B1b0 private
signed-artifact staging implementation in `src/server_staging_lab.rs`.
Core's existing `server_staging_lab` API re-exports that implementation
through the default-off Linux feature `server-bootstrap-b1b-custody-lab`.
Both use the same B1a release verifier and its exact types. The standalone
package still builds and tests independently; it does not depend on Core.

On Linux, run both original acceptance suites:

```sh
cargo test --manifest-path modules/bootstrap-b1b0/Cargo.toml
cargo test --manifest-path aquilum-app/Cargo.toml --locked -p aquilum-core \
  --features server-bootstrap-b1b-custody-lab --test server_bootstrap_b1b0
```

The existing B1b0 workflow also verifies Core type identity, downstream
B1b1 approval compatibility and the default Core build. The signed artifact,
approval, ownership, symlink, crash refusal and idempotence contracts are
unchanged. No new installer, SSH, systemd, File Gate or release authority
is introduced. The package cannot execute, install, activate or authorize
a Node binary.
