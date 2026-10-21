# Node Catalog D1.5 — isolated parity stage

This package is **not yet wired into Akula Core**. The Linux catalog implementation
is copied byte-for-byte from `aquilum-app/core/src/node_catalog_lab.rs`.
Its existing filesystem and symlink-adversary tests are copied, with only the
feature gate and import path changed.

This crate contains an independently typed `VaultId` with the same UUID
representation and string/serde behavior as `Core::sync::VaultId`, but
it cannot grant custody. An eventual Core wrapper must translate at the
trusted boundary using `VaultId::from_uuid` and `as_uuid` with tests
proving stable object IDs and no authorization bypass.

Run:
`cargo test --manifest-path modules/node-catalog/Cargo.toml`

Next: Core adapter as a separate PR, then CI path-filter changes as another
PR. Do not remove required Application checks, D2 host tests, or defaults.
