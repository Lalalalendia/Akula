# PC-FG-03 — history revision identity and exact restore

**Task:** FG-04, immutable revision identity, BLAKE3, exact restore/revert.

**Notion:** [PC-FG-03](https://app.notion.com/p/40332a84beec8182900af091e5cc6814)

**Disposition:** reproduced on the local Windows workstation using a synthetic note. This report covers an additional, uncommitted native-test run; the separate Draft PR #707 is tracked independently. No merge, installed-UI acceptance, or physical-device testing is claimed.

## Change

Extended `aquilum-app/native/src/history/tests.rs::restore_and_revert_rewrite_note`. The test now records the selected source revision ID and BLAKE3, reverts it, and restores the exact selected revision. It verifies that revert and restore create distinct revision IDs and that the source and intervening revision IDs continue to resolve to their original bytes and hashes. No production history behavior was changed.

The fresh run printed these fixture identities and BLAKE3 values:

| Revision | ID | BLAKE3 |
| --- | --- | --- |
| Selected source | `1792847724857_45a944bf_me.md` | `a569c27058dbcc6982c55afe0410c5ba1f3aa1a0d6eef69b6b46c1c356e611b3` |
| Current before revert | `1792847725131_45a944bf_agent.md` | `6c318a2c2218efa84498992b7e49eeb5b779ac82c392f7fc8520e4dd3482eda5` |
| New revert revision | `1792847725146_45a944bf_revert-1792847724857.md` | `a789a3743e3c55e5d4ace5ed3382669ce24abbc6c91054f4bb62bdeecc81e01e` |
| New restore revision | `1792847725160_45a944bf_restore-1792847724857.md` | `a569c27058dbcc6982c55afe0410c5ba1f3aa1a0d6eef69b6b46c1c356e611b3` |

The restored bytes match the selected source hash. The revert and restore revisions have new IDs, and the selected source and revert revisions remain addressable by their original IDs. The latest run also denied both operations for a nonexistent revision ID without changing the note or history, and a repeated exact restore was a no-op.

## Checks

The fresh run used Rust `1.97.1`, locked dependencies and offline mode. It used a dedicated local temp directory with a DACL restricted to the current user, SYSTEM, and Administrators. Cargo used the workspace-local home `D:\mobakula\.artifacts\one-pc-cargo-home` and the existing target `D:\mobakula\.artifacts\cargo-native`.

```powershell
$env:TEMP=$env:TMP='D:\mobakula\.artifacts\one-pc-checks\PC-FG-01-20261024-current\restricted-temp'
$env:CARGO_HOME='D:\mobakula\.artifacts\one-pc-cargo-home'
$env:CARGO_TARGET_DIR='D:\mobakula\.artifacts\cargo-native'
$env:CARGO_INCREMENTAL='0'
$env:CARGO_BUILD_JOBS='1'
cargo +1.97.1 test --manifest-path D:\mobakula\aquilum-app\Cargo.toml --locked --offline -p aquilum-native --bin aquilum-native history::tests::restore_and_revert_rewrite_note -- --exact --nocapture --test-threads=1
```

- `history::tests::restore_and_revert_rewrite_note` — **1 passed**, 761 filtered out.
- Runtime marker: `unknown_id_no_write=true repeated_restore_noop=true exact_id_restore=PASS`; restored BLAKE3 equals the selected source; revert and restore IDs are distinct.
- `rustfmt +1.97.1 --check --edition 2021 native/src/history/tests.rs` and `git diff --check` — **PASS**.

Log: `.artifacts/one-pc-checks/PC-FG-03-20261024-current/restore-revert-history-final.log`, SHA-256 `2E3E4465739FE895BFB504E459F30D7A8F5CBAD958838058898CE32183E555C3`.

## Limits and repository state

This is a native unit test over a synthetic note on one Windows PC. It does not claim installed-UI, GitHub Actions, merge/Main PR gate, or separate physical-device acceptance. The existing Draft PR #707 is a separate core integration-test snapshot; this uncommitted native-test change is not part of that PR and its local result does not establish PR #707 CI.

- Branch: `main`.
- Exact `HEAD`: `530fe3d6794ee28d2c7098af1266ff5340903543` (unchanged).
- This local native-test patch and local report copy remain uncommitted; unrelated existing local changes were preserved. Draft PR #707 is a separate test-only change.
- At publication time, the Notion card tracks the existing Draft PR #707 at `90c3be0234332766ac177016adf89dd50fb831cf`; exact-head CI and review remain open.

**Next step:** complete exact-head checks and review for PC-FG-03 Draft PR #707. PC-FG-04 follows after PC-FG-03's required acceptance; its local Windows recheck is recorded separately and PR-only acceptance remains open.

## Publication state — 2026-10-24

- The latest local native test result, BLAKE3 identities, negative cases, and log hash are recorded in the PC-FG-03 Notion card and this report. The uncommitted native-test patch is not included in a GitHub source PR.
- This evidence report is published in the documentation-only [PR #699](https://github.com/Lalalalendia/Akula/pull/699) at `docs/one-pc-plan/PC-FG-03-history-revision-identity.md`; that PR does not contain the PC-FG-03 source change.
- The separate test-only [Draft PR #707](https://github.com/Lalalalendia/Akula/pull/707) is open at `90c3be0234332766ac177016adf89dd50fb831cf`. Application checks run [#40096716276](https://github.com/Lalalalendia/Akula/actions/runs/40096716276) was queued when checked; exact-head CI and review are not yet accepted. No PR was merged.
- The native test patch and unrelated pre-existing local changes remain in the worktree at `530fe3d6794ee28d2c7098af1266ff5340903543`.

## CI follow-up — 2026-10-24T13:17:50Z

The exact-head Application checks for PR #707 now show the `history_revision_identity_contract` test passing (1/1) and the separate Native frame fidelity job passing. The Required Rust tests and policy checks job failed in an unrelated macOS updater transaction test: `updater::macos_transaction::tests::fresh_user_install_has_one_shared_lock` could not reacquire its BSD lock (`Resource temporarily unavailable`, OS error 11). That job reported 731 passed, 1 failed, and 66 ignored. The Main PR gate remained queued, so exact-head acceptance was still open at this check. PR #707 remained Draft and unmerged; no retry was dispatched.

