# RELEASE-LINUX-RUNTIME-GATE-01 — Linux updater compiled, unsafe overwrite removed

Status: **native runtime wired for denial only; public and automatic release still HOLD**.
Canonical plan: [AKULA-MASTER-INDEX-01](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b).

## Why this change exists

The previous installed Linux updater in `aquilum-app/native/src/updater/apply.rs` used `std::fs::write` to an image-adjacent `.update` file, then renamed it over the live AppImage and exited. The old executable was lost before any proof that the new version could render its GUI. This path was reachable by direct calls to `apply()`, irrespective of release-channel UI visibility.

Now `apply.rs` delegates to `updater::linux_transaction::guarded_runtime_apply()`. The original direct-overwrite/exit operation is gone. The copied, crash-tested transaction engine is compiled from **`aquilum-app/native/src/updater/linux_transaction.rs`**, rather than a standalone script. It retains stable OS `flock`, bounded candidate staging, stage receipt, directory `fsync`, previous executable retention, controlled cold recovery, and fail-closed ambiguous stages.

**Installed apply entrypoint refuses every candidate before it can stop Akula or acquire a lock, write, rename, spawn, or exit.** This is intentional: `channel_ready()` is still false, `PUBLIC_KEY` still empty, and there is no Linux GUI startup ACK. Merely enabling `channel_ready()` later cannot silently restore the old destructive path or authorize these transaction primitives.

## Exact acceptance

The existing `Akula Linux AppImage rollback lab` workflow now compiles the **native module source** as a standalone Rust test harness with `rustc --edition=2024 --test -D warnings` and runs 11 native tests, including three real `SIGKILL` child-process windows and a direct guarded-runtime refusal test. The same module is in the native crate and receives full Application CI compile/test coverage. CI additionally proves:

- Linux `apply()` delegates to the production HOLD adapter and no longer contains the direct `rename(&fresh,&image)` overwrite;
- a direct installed invocation with forged bytes rejects without invoking `before_exit()`;
- old Aquilum release remains HOLD, `PUBLIC_KEY = ""` and `channel_ready() = false`.

## Missing, explicitly separate gates

1. Secure publisher root (Akula-owned private key offline; pinned public key shipped with app). The current Minisign lab is **not** publisher proof.
2. Bind exact signed candidate bytes, product/platform, source SHA, version floor and digest to a typed, non-forgeable installed admission before any `Gate::prepare`; after-copy hash verification and immutable file authority are needed.
3. Generation-specific Linux startup ACK emitted after real first rendered GUI frame, with process identity and previous version retained until durable ACK. Spawn success is not ACK.
4. Real installed v1 -> v2 / v2 failure -> v1 end-to-end and interruption tests on Linux, then power-loss experiments and physical QA.

No release is published; no macOS/Windows updater change; no Aquilum path touched; destructive Sync remains disabled. This is **not** a claimed production-ready updater.
