# Main-branch Rust cache and sync-only PR CI

Status: implementation in review (2026-10-17).

## Root cause from real logs

`Application checks` previously ran only on `pull_request`; the rust-cache action wrote roughly 915 MB archives into pull-request merge-ref scopes that sibling PRs cannot restore. The full `Main PR gate` frequently showed `No cache found`. In run 38989730748, the workspace took roughly 2m48s to compile and the 464 native tests took roughly 4m16s to execute. A compiler cache cannot remove the native tests' execution cost.

## Proposed behavior

- The required `Main PR gate` remains present for every PR. PRs restore the shared main cache, but set `save-if: false` to avoid repeated private 900 MB uploads.
- After pushes changing `aquilum-app/**` or `app-checks.yml`, a separate job on `main` compiles all workspace tests (without executing them) and sync-engine tests only when there is no exact shared cache hit. It saves that default-branch cache for other PRs.
- Both jobs share the same cache key and Cargo target directory; workspace crate caching is temporarily enabled and must be monitored for size/quota effects.
- CI scope is deliberately conservative: changes exclusively to `core/src/sync/**` or `core/tests/sync_contract.rs` run Core tests with and without `sync-engine`, without unrelated native UI tests. Any other Rust or workflow changes run the full test suite. Docs-only changes skip Rust.
- Any test-scope detection error fails CI rather than bypassing mandatory tests. Required check name stays `Main PR gate`.

## Verification before claiming a speedup

1. The workflow-change PR must pass the full `Main PR gate`.
2. After merging, verify that the `main` push seeded a cache successfully.
3. In a subsequent sync-only PR confirm real cache restoration and both Core test modes; verify native UI tests are skipped only there.
4. Measure warmed PR wall time versus previous p50/p90. Cache hits save compilation time, not heavy native test runtime.

Security and full release/OS matrices remain unchanged. Do not activate sync writes based on this CI scope change.