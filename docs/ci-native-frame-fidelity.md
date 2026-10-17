# Native frame-fidelity CI sharding

Status: active proposal; must be validated before merging. Created 2026-10-17.

## Why

The warm, unsharded Akula native test binary ran for ~240 seconds in Actions run 38992330822; the seven `app::tests::perf::frames::*` checks are computationally expensive. They are essential pixel-equivalence assertions and **must not be disabled**.

## Diagnostic experiment

On PR #66, initial diagnostic run 38992862841 independently ran:

- `cargo test --locked -p aquilum-native --bin aquilum-native -- app::tests::perf::frames::`: 7 passed, 0 ignored, 457 filtered; test runtime 100.37 seconds, ~135 seconds including build/check.
- `cargo test --locked -p aquilum-native --bin aquilum-native -- --skip app::tests::perf::frames::`: 414 passed, 43 ignored, 7 filtered; runtime 98.00 seconds, ~128 seconds including build/check.
- Together both groups cover 464 native test names (421 executed, 43 intentionally ignored), without changing or weakening assertions.

## Production CI design

One lightweight `classify` job selects none, sync-only or full workspace. The `required-tests` job retains all baseline installer, plugin boundary, architecture tests, patch whitespace, and Rust tests. For workspace scope it runs `cargo test --workspace --locked -- --skip app::tests::perf::frames::` followed by Core sync-engine tests. A separate job `native-frames` runs the full seven frame-fidelity tests, in parallel. Both jobs restore the default-branch Cargo cache (never reupload it).

The single protected status `Main PR gate` is a final aggregator. It uses `always()` and requires classification and required-tests success; for workspace scope it also requires native-frames success, whereas sync/none require the native lane skipped. Any missing or unexpected result fails closed. A cancelled, failed or skipped workspace frame lane must block the merge.

Important: sharding accelerates wall-clock latency at the cost of an additional concurrent GitHub runner when native scope changes. It is not a claim of lower billed compute minutes or GPU speed. Rust doctests and non-frame unit/integration tests remain in the workspace suite. The `Protect main` ruleset requires the same exact check `Main PR gate`.

## Verification and rollback

1. Required `Main PR gate` runs and succeeds on the workflow-edit PR, with both required lanes successful.
2. Confirm the skipped frames reported by the first job equal those executed in the second; no missing native test names.
3. Compare full workflow elapsed time and runner minutes against previous warm full UI PRs. If the benefit is small or flaky, revert sharding while preserving tests.
4. Run a sync-only PR through the same aggregator; the native fidelity job must be skipped, not required.

Example diagnostic source: https://github.com/Lalalalendia/Akula/actions/runs/38992862841

## Production validation

On [PR #66](https://github.com/Lalalalendia/Akula/pull/66), both required lanes and the final fail-closed `Main PR gate` completed successfully in [run 38993219815](https://github.com/Lalalalendia/Akula/actions/runs/38993219815). The full workflow elapsed **290 seconds (4m50s)**, compared with **387 seconds (6m27s)** for the recent warmed unsharded run [38992330822](https://github.com/Lalalalendia/Akula/actions/runs/38992330822), about 25% less wall time. The runs were on different revisions; keep this as a preliminary comparison until repeated measurements are available.

The native remainder job passed 416 and ignored 43 tests, with 7 filtered. The separate frame-fidelity job passed all 7. Together they cover all 466 native tests at this revision. The workspace and sync-engine tests also passed. The full CI remains necessary on non-sync changes, and the additional runner may increase billed minutes.

This documentation-only follow-up PR intentionally probes `mode=none`: `Classify required CI lanes` and `Required Rust tests and policy checks` must pass, `Native frame fidelity` must be skipped, and the final `Main PR gate` must succeed without running Rust tests.
