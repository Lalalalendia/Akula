# PC-FG-03 — history revision identity and exact restore

**Task:** FG-04, immutable revision identity, BLAKE3, exact restore/revert.  
**Notion:** [PC-FG-03](https://app.notion.com/p/40332a84beec8182900af091e5cc6814)  
**Disposition:** freshly reproduced on the current worktree with a synthetic note on one Windows PC. Notion, GitHub, and PR state were not changed.

## Change

Extended `aquilum-app/native/src/history/tests.rs::restore_and_revert_rewrite_note`. The test now records the selected source revision ID and BLAKE3, reverts it, and restores the exact selected revision. It verifies that revert and restore create distinct revision IDs and that the source and intervening revision IDs continue to resolve to their original bytes and hashes. No production history behavior was changed.

The fresh run printed these fixture identities and BLAKE3 values:

| Revision | ID | BLAKE3 |
| --- | --- | --- |
| Selected source | `1792829956885_5cf09cc8_me.md` | `a569c27058dbcc6982c55afe0410c5ba1f3aa1a0d6eef69b6b46c1c356e611b3` |
| Current before revert | `1792829956899_5cf09cc8_agent.md` | `6c318a2c2218efa84498992b7e49eeb5b779ac82c392f7fc8520e4dd3482eda5` |
| New revert revision | `1792829956912_5cf09cc8_revert-1792829956885.md` | `a789a3743e3c55e5d4ace5ed3382669ce24abbc6c91054f4bb62bdeecc81e01e` |
| New restore revision | `1792829956924_5cf09cc8_restore-1792829956885.md` | `a569c27058dbcc6982c55afe0410c5ba1f3aa1a0d6eef69b6b46c1c356e611b3` |

The restored bytes match the selected source hash. The revert and restore revisions have new IDs, and the selected source and revert revisions remain addressable by their original IDs.

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

- `history::tests::restore_and_revert_rewrite_note` — **1 passed**, 751 filtered out.
- Runtime marker: `exact_id_restore=PASS`; restored BLAKE3 equals the selected source; revert and restore IDs are distinct.
- `git diff --check` — **passed** after updating this report.

Log: `.artifacts/one-pc-checks/PC-FG-03-20261024-current/restore-revert-history-retry.log`.

## Limits and repository state

This is a native unit test over a synthetic note on one Windows PC. It does not claim installed-UI, GitHub Actions, merge/Main PR gate, or separate physical-device acceptance. The relevant workflows were not dispatched; no PR was created or updated.

- Branch: `main`.
- Exact `HEAD`: `530fe3d6794ee28d2c7098af1266ff5340903543` (unchanged).
- PC-FG-03 source and this report remain uncommitted; unrelated existing local changes were preserved.
- Notion status was left as-is (`В очереди`).

**Next plan task:** PC-FG-04 — symlink/junction/reparse handling and path replacement race. It depends on PC-FG-01, whose current local tests passed; revalidate PC-FG-04 against the current source before relying on its older report.



---

## Publication update — 2026-10-24

This report has been published in the documentation-only one-PC evidence PR [#699](https://github.com/Lalalalendia/Akula/pull/699). The report records local evidence and its stated limits; this documentation PR does not include the underlying application source changes. PC-FG-01 implementation remains in Draft [#703](https://github.com/Lalalalendia/Akula/pull/703); PC-FG-02 test code/report are in Draft [#704](https://github.com/Lalalalendia/Akula/pull/704). No PR was merged.
