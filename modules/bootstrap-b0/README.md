# Bootstrap B0 module — isolated compatibility experiment

Source and all existing integration tests were copied without changing the behavior.
The original `aquilum_core::server_bootstrap_lab` remains the active implementation.
There is no SSH client, installation command, credential access or write authority here.

Run `cargo test --manifest-path modules/bootstrap-b0/Cargo.toml`.

Gate A1: standalone tests pass and API/behavior match the original.
Gate A2 (separate PR after A1): change Core to a compatibility re-export,
update workspace Cargo.lock with Cargo, rerun legacy Core B0 and Main PR checks.
Gate B (separate PR): narrow B0 workflow paths, keep mandatory Core full fallback
for shared Cargo.toml/lib.rs/Cargo.lock changes; measure fanout before/after.
