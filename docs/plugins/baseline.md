# PLUGIN-00 baseline and measurement protocol

Status: initial evidence collected 2026-10-17; values below refer to the explicitly cited commits and workflows, **not** a plugin-enabled Akula build.

## Source state and reproducible commands

- Repository: [Lalalalendia/Akula](https://github.com/Lalalalendia/Akula)
- Captured main SHA: `939d5109f19a990e69cc2cbfbc1f046d941e9766`
- Current baseline: **no plugin host, plugin runtime, plugin manager or permission store**. Built-in Core services and sync contracts exist independently.
- Linux workspace test command (existing Main PR gate, from `aquilum-app/`): `cargo test --workspace --locked`
- Feature test: `cargo test -p aquilum-core --features sync-engine --locked`
- Architecture test (Node.js built-in only): `node --test .github/scripts/plugin-architecture.test.cjs` from repo root.
- Existing cross-platform manual workflow: [Native check](../../.github/workflows/native-check.yml), `workflow_dispatch` with `ref` and `platforms`; it tests Linux/macOS/Windows by request, not on every PR.

## Evidence already available before this plugin PR

- [Successful Application checks run 38989730748](https://github.com/Lalalalendia/Akula/actions/runs/38989730748) on SHA `94262ce0fd3949c9006af413cbd4fcf38988c31d`: checkout, architecture-budget test, installer checks and **Run Rust tests** finished successfully. This is a historic pre-plugin run on a different PR revision; it does **not** validate our future runtime.
- [Native watcher/search run 38989423789](https://github.com/Lalalalendia/Akula/actions/runs/38989423789) on SHA `fcf06c3eaaf84a0d9255e70365c3a84ad5bf904c`: the existing **Run Rust tests** step passed. Different SHA, not a plugin feature test.
- [docs/engineering-budgets.md](../engineering-budgets.md) audited 90 successful earlier Application checks: median about **7.0 minutes**, p90 about **8.8 minutes**, in the short available 16–17 October sample. Not an application startup/runtime measurement.
- The currently required `Main PR gate` automatically runs both Rust commands when a PR changes `aquilum-app/` or the workflow itself. There is no new mandatory status check for PLUGIN-00.

## Measurements not yet obtained (explicit, do not invent values)

| Metric | Pre-plugin numeric value | Repeatable method | First gate |
| --- | --- | --- | --- |
| Native cold startup (ms) | **Not measured here** | Build same release configuration for a pinned SHA, run fixed sample vault, capture first visible native window/metrics on each OS | PLUGIN-00 follow-up / PLUGIN-05 |
| Peak startup RSS (MiB) | **Not measured here** | OS-process sampled peak RSS during cold/warm launch; separate native and plugins; repeat median and p95 | PLUGIN-00 follow-up / PLUGIN-05 |
| Note save latency (P50/P95) | **Not measured here** | Same file sizes and synthetic vault; measure DocumentHub + File Gate commit end-to-end, verify history/hash/readback | PLUGIN-00 follow-up / PLUGIN-09 |
| Plugin RPC and cold guest load | **Not applicable: no guest runtime** | Measure after a runtime PoC; report guest package bytes, host RSS, p95 host-call | PLUGIN-05 |
| CI exact PR-head regression | **Pending until PR runs** | Existing Main PR gate on exact head commit, compare baseline and provide link | PLUGIN-00 |

## Protocol: fair before/after comparison

1. Record commit SHA, OS/image, build profile, Rust toolchain, CPU architecture, sample vault hash/size and number of documents. Separate fresh cache from warm cache.
2. For native launch, run the existing `Native check` workflow manually on a specific ref with release build/window launch, or an equivalent reproducible local harness; report actual wall-clock / process RSS and variance. The current workflow measures macOS window visibility and optionally emits native metrics, but **not** an identical ready-made cross-platform RSS dashboard.
3. For save, seed a fixed Markdown sample under a temporary vault and app-data dir; assert bytes/history/expected revision after save; measure at least repeated steady-state operations after one warmup. Keep outlier/runs, do not invent latency.
4. At PLUGIN-05, run side-by-side candidate runtimes using the **same** test guests, permissions, vault and hardware, then decide Wasmtime vs Extism by evidence.
5. At PLUGIN-21, measure 0/1/10/100 enabled extensions and all-disabled mode; if plugin-disabled build regresses significantly, revert or justify it explicitly with benchmark evidence.
6. Archive raw metrics, commands and CI URLs next to PR/issue; do not treat pass/fail alone as evidence of speed.

## Baseline constraints

- This stage deliberately changes **no Rust source and no Cargo dependencies**. Existing Rust behavior should be preserved.
- A successfully green `Main PR gate` on the PLUGIN-00 PR demonstrates the old Rust test commands still pass, but does not prove security of an unimplemented runtime.
- The program does not duplicate [SYNC PR 00–11](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11).
