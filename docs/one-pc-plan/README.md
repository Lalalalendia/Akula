# Akula one-PC evidence

This folder collects the local execution evidence for [AKULA-ONE-PC-PLAN-01](https://app.notion.com/p/40332a84beec8199b005f7a239210763). Each report states its input/source SHA, local commands, results, and acceptance limits.

## Reports

- Environment and setup: [PC-00.01 baseline](PC-00.01-environment-baseline.md), [repository inventory](PC-00.01-repository-inventory.md), [toolchain/build](PC-00.02-toolchain-build.md), [synthetic fixtures](PC-00.03-synthetic-fixtures.md), [two local instances](PC-00.04-two-local-instances.md), [local checks](PC-00.05-local-checks.md), [main/PR audit](PC-00.06-main-and-pr-audit.md).
- Desktop: [PC-DESK-01](PC-DESK-01-offline-markdown-history.md), [PC-DESK-02](PC-DESK-02-workspace-switch-save-guard.md), [PC-DESK-03](PC-DESK-03-100k-editor-undo-redo.md).
- File Gate: [PC-FG-01](PC-FG-01-interprocess-filegate.md), [PC-FG-02 code and report in Draft #704](https://github.com/Lalalalendia/Akula/pull/704), [PC-FG-03](PC-FG-03-history-revision-identity.md), [PC-FG-04](PC-FG-04-path-reparse-race.md), [PC-FG-05](PC-FG-05-confirmed-import-file-gate.md), [PC-FG-06](PC-FG-06-negative-file-gate.md).
- Network/dependencies: [PC-NET-01](PC-NET-01-locked-interop.md), [PC-PERF-09](PC-PERF-09-rustsec-dependency-audit.md).

## Publication scope

Draft PR [#699](https://github.com/Lalalalendia/Akula/pull/699) publishes evidence documents and one-PC helper tooling only. It does not publish application source changes for the local A1 tasks. File Gate implementation PRs are tracked separately: PC-FG-01 Draft [#703](https://github.com/Lalalalendia/Akula/pull/703), PC-FG-02 Draft [#704](https://github.com/Lalalalendia/Akula/pull/704). These PRs are not merged, and no physical second-device, external VPS, or power-loss acceptance is implied.
