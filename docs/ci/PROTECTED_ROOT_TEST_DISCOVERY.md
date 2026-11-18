# Protected CI repair: no skips, exact compiled ELF

GitHub-hosted Linux root CI jobs were compiling an actual `server_bootstrap_*` test executable under Cargo's build artifact directory but searched only `debug/deps/`. This left count=0 while compiled tests never ran, and correctly blocked the main PR gate.

The new `scripts/ci/resolve_cargo_root_test.py` re-runs the exact `cargo test --locked --no-run --message-format=json` target with its required lab feature. It accepts exactly one Cargo compiler-artifact test binary, verifies its resolved location is a regular executable inside the dedicated build tree, rejects symlinks or ambiguity, and never runs anything itself. Every original `sudo` root test remains required.

B1d0 additionally replaces a collision-prone fixed scratch directory with an isolated fresh root-owned 0755 directory, mounted only inside its existing private mount namespace. It retains strict root ownership and the original safety checks on `/opt`. No production path is mutated by the workflow outside that namespace.

P2P lab: one `valid_label` branch is simplified to an equivalent Boolean condition to resolve a dedicated Clippy diagnostic, without weakening reserved-name rules.

All required lanes and the fail-closed gate remain enabled. Validate actual Linux root/FD, P2P lint and b1d0 tests before merging. This PR changes CI infrastructure only, not the Akula Languages user-facing feature.