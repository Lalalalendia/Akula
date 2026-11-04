# PC-NET-01 — locked interop graph and two-process evidence

Checked: 2026-10-24. This report records local work for [AKULA-ONE-PC-PLAN-01](https://app.notion.com/p/40332a84beec8199b005f7a239210763), task [PC-NET-01](https://app.notion.com/p/40332a84beec81429a96e4479544df14). Notion, GitHub branches, PR metadata, and remote refs were not changed.

## Input and current Git state

- Local repository: `D:\mobakula`; branch `main`; input HEAD `530fe3d6794ee28d2c7098af1266ff5340903543`, matching `origin/main` at inspection.
- The worktree was already dirty before this task. Existing File Gate, P2P identity/inbox, search, and other plan work was preserved.
- There is no matching Roha transport implementation in the current repository. The existing `interop/akula-staya` process test is the only real QUIC/Bao Akula–Staya integration found in this checkout.

## Changes made

- Added a separate lockfile at `interop/akula-staya/Cargo.lock`. It belongs to the standalone integration crate and does not replace or alter `aquilum-app/Cargo.lock`.
- Updated `.github/workflows/p2p-akula-staya-real.yml` to require the lockfile and pass `--locked` to every Cargo build, test, check, and Clippy invocation for the Akula bridge. The separately pinned Staya release build already used `--locked`.
- Added the new lockfile to both RustSec paths: the dependency watchdog and the blocking Application PR gate. The dependency-gate test now explicitly covers this lockfile.
- Updated the interop README and manifest comments to document the separate lock boundary.

## Resolved dependency matrix

| Component | Resolved value | Source / evidence |
| --- | --- | --- |
| Akula interop package | `akula-staya-integration 0.1.0`, MSRV 1.91 | `interop/akula-staya/Cargo.toml` |
| Lock generation toolchain | `rustc 1.94.1`, `cargo 1.94.1` | Local toolchain; Cargo selected versions compatible with Rust 1.91 |
| `staya-transfer` | `0.1.0`, full commit `ccfe82dc6125e2810e95043a6a4ac55954b710e0` | Git source is pinned in the manifest and lockfile |
| Akula bridge `iroh` | `1.3.0` | crates.io in the new lockfile |
| Akula bridge `noq` / `noq-udp` | `1.4.0` / `1.4.0` | crates.io in the new lockfile |
| Roha | absent | No Roha package or Git source in the current integration manifest/lockfile |
| Staya preview binary lock | `iroh 1.3.0`, `noq 1.3.0`, `noq-udp 1.3.0` | Pinned Staya source lock at blob `9f12c758013719dbc74eecba074fab2e60a83d72` |

This records what the code actually resolves; it does **not** prove the planned Roha + forked-noq source matrix. The two test endpoint binaries use the same pinned Staya release lock. The Akula bridge is a parent-side consent/queue adapter and does not open the QUIC endpoint itself.

The proposed fork revisions are not safe to substitute yet: noq PR [#7](https://github.com/calex794/noq/pull/7) remains Draft and its exact-head `CI`, `iroh-patchbay`, and Wine jobs were cancelled; Roha PR [#11](https://github.com/calex794/roha/pull/11) remains Draft and its exact-head `CI` failed in `cargo deny`, while its Patchbay and platform tests were cancelled. The source roadmap says not to switch to the noq PR SHA before full noq CI and a separate locked Roha/Akula integration. No second transport or unverified fork was introduced.

## Process-test evidence

Existing workflow run [#40033078715](https://github.com/Lalalalendia/Akula/actions/runs/40033078715) ran at head `7a37fc45b27d70b937c6e424da2c749d23c80a11`. Both `windows-2022` and `ubuntu-24.04` jobs completed successfully, including **“Live two-process direct QUIC plus independent physical Bao output.”** This is a two-process simulation on one runner, not acceptance on two physical computers.

At committed-HEAD level, the interop source and workflow were unchanged between merge commit `c899d8365cc5226ca38375e05072e86cd275dd55` and the current `main` HEAD. This task now adds local edits to those files, and the worktree also contains uncommitted Core P2P identity/inbox and File Gate changes used by this path. Therefore the older green run is historical evidence, not a pass for the exact current worktree or the newly locked workflow.

## Local verification

- `cargo generate-lockfile --manifest-path interop/akula-staya/Cargo.toml` — exit 0; Cargo locked 499 packages using the crate's Rust 1.91 compatibility floor.
- `cargo audit --no-yanked --file interop/akula-staya/Cargo.lock` — exit 0; no vulnerabilities. Three allowed unmaintained-crate warnings remain: `atomic-polyfill`, `paste`, and `smallstr`.
- `node --test .github/scripts/dependency-security-gate.test.cjs` — 7 passed, 0 failed.
- `cargo metadata --manifest-path interop/akula-staya/Cargo.toml --locked --offline --format-version 1 --no-deps` — exit 0; this checks manifest metadata only, not full dependency resolution or compilation.
- PyYAML parsed the three edited workflows successfully; `git diff --check` — exit 0.
- An offline `cargo tree` attempt stopped because the local Cargo cache lacks the `adler2 2.0.1` source archive. No local QUIC/Bao build or test was claimed.

## Remaining acceptance

PC-NET-01 is **not complete**. The lock and CI now make the existing Iroh/Staya integration reproducible in principle, but the planned Roha/forked-noq source matrix is absent, the fork PR gates are not green, and the updated workflow has not run on this exact dirty worktree. No claim is made for physical two-PC acceptance, the installed Akula UI, or a production transport. Push, PR updates, merge, and Notion writes remain untouched under the current instruction.

Lockfile SHA-256: `CDE946AABB7330ABFE03AE6E990991BDEF1347F95B5E2F7D847F4C6792BBFFB6`.

Local logs are under `.artifacts/one-pc-checks/PC-NET-01-20261024-lockgen/`:

- `generate-lockfile-escalated.log` — SHA-256 `9A0BBDA404F0CEA73AA840F1FCEDF4C3CA4FABCBEDF24E01FA7BB81D721C13D2`.
- `cargo-audit-final.log` — SHA-256 `E3DD2309B7875ED4FF2EACB6C95ADD8C515167134278A1D135E8FA7B19ABDB50`.
- `dependency-security-gate-tests.log` — SHA-256 `409A619FD72165462906458F58EE67B17EEACE59E34E78EF973EEDF886CD8652`.



---

## Publication update — 2026-10-24

This report has been published in the documentation-only one-PC evidence PR [#699](https://github.com/Lalalalendia/Akula/pull/699). The report records local evidence and its stated limits; this documentation PR does not include the underlying application source changes. PC-FG-01 implementation remains in Draft [#703](https://github.com/Lalalalendia/Akula/pull/703); PC-FG-02 test code/report are in Draft [#704](https://github.com/Lalalalendia/Akula/pull/704). No PR was merged.
