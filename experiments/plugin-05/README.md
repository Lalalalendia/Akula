# PLUGIN-05 — isolated runtime comparison (research only)

Canonical scope: Akula PLUGIN-00–22, Phase B / PLUGIN-05.
Spec: https://app.notion.com/p/3fc32a84beec81988742d38ed545bbb7

Do not enable this code in the Akula application. Experiments are separate
Cargo projects OUTSIDE aquilum-app/. They have no live PermissionBroker,
no filesystem grants or real network access. Only synthetic guest input.

## 05A — Wasmtime direct embedding

First slice uses Wasmtime 50.0.0, minimal crate features and fixed WAT fixtures.

    cargo test --manifest-path experiments/plugin-05/wasmtime/Cargo.toml
    cargo build --release --manifest-path experiments/plugin-05/wasmtime/Cargo.toml
    experiments/plugin-05/wasmtime/target/release/akula-plugin05-wasmtime

Expected successes: hello=42 and synthetic host-mediated scoped_note(7)=17.
Expected denials: other token returns -1; nonregistered WASI socket import
is rejected; looping guest exhausts fuel; memory.grow beyond 1 MiB fails.

Note: scoped_note is a synthetic test oracle and does NOT issue actual Akula
vault grants or call production permission-checking code.

## Reproducibility and comparison

1. Record exact guest fixture hashes, rustc version, OS/CPU, lockfile,
   compilation profile and warmup count.
2. Measure construction/compile separately from instantiate and hostcalls.
   Cold compile must not be mislabeled cold startup of installed Akula.
3. The standalone exe byte count is not installer growth.
4. Linux VmHWM is whole-process high-water RSS, NOT guest linear memory.
5. Extism uses Wasmtime underneath. 05B compares ABI overhead, SDK and
   packaging, NOT separate WebAssembly execution engine security.
6. Equivalent fixtures for Extism (05B) and Component/WIT (05C) must run
   on the same hardware, with ABBA candidate order, before runtime selection.
7. Complete acceptance needs Ubuntu/Windows/macOS tests, process memory
   budgets, timeout/cancellation, revoke/teardown and real host-owned Broker.
8. This branch is research-only. No production linkage, new plugin grants,
   installer changes, ABI promises, or release status are implied.

## First fixture acceptance

| Control | Expected result |
| --- | --- |
| Hello | 42 |
| Synthetic allowed read | Token 7 -> 17 |
| Synthetic denied read | Other tokens -> -1 |
| WASI socket import | Instantiation rejected |
| Infinite guest | Fuel trap; host survives |
| Memory growth | Over-limit growth rejected |
| Footprint | Standalone binary bytes and Linux process VmHWM |
| Latency | Compile, instantiate and 1000-call p50/p95 |
| Default Akula | Cargo manifests and source remain untouched |

CI: .github/workflows/plugin05-wasmtime-lab.yml is BRANCH-ONLY, opt-in
and nonrelease; no new required status check or main application change.
