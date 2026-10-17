# Akula architecture and CI budgets

Status: **advisory rollout** (17 October 2026). None of these numerical budgets is an additional required status check.

## Verified initial observations

The initial audit inspected 19 GitHub Actions workflow files and the available first 390 workflow runs (repository history from 16-17 October 2026). Successful Application checks had median ~7.0 min and p90 ~8.8 min (90 samples). There were 130 PR branch/SHA pairs with median three and maximum five workflows. These numbers are a short, nonrepresentative baseline, **not** a validated production SLO.

The Git tree had 892 own Rust files under core/native (non-vendor) plus 315 vendored Rust files. Vendored dependencies, generated files, test files, fixtures and intentionally isolated performance harnesses must not count toward production module-size drift warnings.

## Fast PR gate: architecture delta

`.github/scripts/architecture-budget.cjs` is run from the existing `Main PR gate`. It compares Git objects at the PR base and merge HEAD, and reports changed own-production Rust modules: source lines, reference breadth to root `crate::` modules, public re-exports, new child modules and new direct dependencies in core/native Cargo.toml. The output appears in the GitHub Actions step summary.

Initial review thresholds (all **non-blocking**):

- A new production Rust source file with more than 600 lines, or an existing >800-line file growing by at least 150 lines.
- An increase of at least 2 distinct root-module references beyond 8, or at least 3 new re-exports beyond 12, or at least 3 child modules beyond 20.
- Net PR growth over 500 production Rust lines, more than 10 new production modules, or more than 2 added direct dependencies.

Metrics are intentionally heuristic. Parsing Rust source with regular expressions is **not** a real compiler dependency graph, and it does not measure inbound fan-in, runtime call fan-out, cyclic dependencies, or compilation impact. Treat warnings as prompts to review; large necessary changes are permissible.

Tests for this logic run on every PR in the stable `Main PR gate`. A malfunction in the check implementation can fail CI, but exceeding the advisory budgets cannot.

## Weekly Actions budget audit

`.github/scripts/ci-run-budget.cjs` runs only during the existing Monday `Branch hygiene` scheduled audit or manual `audit` dispatch. It fetches GitHub Actions metadata newest-first until the seven-day boundary (up to 12 pages / 1,200 runs), explicitly warning if the cap prevents full coverage. Completed skipped workflow events count as triggered, but not as non-skipped CI workload. Output goes to the workflow's step summary. Its step is explicitly `continue-on-error`; a metrics API outage must not block branch hygiene.

- Record **median and p90 wall-clock duration** for `Application checks` and busiest workflows; flag p90 above 10 min or median above 8 min with >=10 observations.
- Record **triggered distinct workflows** separately from **non-skipped distinct workflows** and **raw run count** (including repeated/cancelled) per PR branch/SHA; flag p90 non-skipped workflow fan-out greater than 3 after >=10 sampled revisions.
- Track failures/cancellations/skips separately. Cancellations are excluded from duration medians but retained in non-skipped fan-out as an upper bound (some may have been cancelled before starting a runner); completed skipped workflows are excluded from non-skipped fan-out.
- Do not sum concurrent workflow durations and call that a PR's critical path. The timestamps include queue latency and do **not** measure billed runner-minutes or exact runner queue delays.

## Avoid needless overhead

PR #50 already adds cancellation of superseded `Main PR gate` runs and makes full Core paired performance workflows selective: `perf/*` or `perf(...)`/`perf:` PR titles or `perf-ci` label. Ordinary code PRs should not trigger both heavy Core benchmarks. Dedicated security, platform and release workflows remain separate and must not be weakened.

No new workflow file or required branch status check is introduced for this monitoring. Maintain the existing full Rust workspace plus `sync-engine` tests in `Main PR gate`.

## After observation

Collect at least 1-2 weeks of data, identify the slowest job steps and redundant builds, then decide which budgets (if any) deserve hard enforcement. Prefer structural rules about forbidden dependencies/cycles and lossless behavior, not arbitrary LOC caps. Do not claim reduced real-world runner cost until after-versus-before data is available.

The existing `Branch hygiene` push-on-main path filter also executes the read-only telemetry step after changes to the analyzer or hygiene workflow, to smoke-test authenticated GitHub Actions API access immediately after merge. No extra workflow is created.
