# PC-FG-04 — path, reparse points, and workspace confinement

Date: 2026-10-24
Repository: `D:\mobakula`
Local `HEAD`: `530fe3d6794ee28d2c7098af1266ff5340903543`
Worktree: already contained unrelated modifications before this task; this report records local, uncommitted evidence only.

## Changes

- Added `aquilum-app/core/src/files/gate/path_guard.rs`. File Gate targets must be below a registered workspace. On Windows, every existing directory from the volume root through the target's parent is opened component-by-component without following reparse points and kept open without delete sharing for the mutation lifetime. A junction or rename of a checked parent is refused or blocked.
- Applied the guard to ordinary write, create, binary create, copy, folder create, and folder-tree creation. Folder trees are created component-by-component. Copies create new destination files rather than opening an existing path for overwrite.
- Made the expected-hash read refuse a reparse/non-regular target leaf. Windows create/copy file handles deny delete sharing while their bytes are being written.

## Verification

All commands ran on this Windows workstation. Test target was on `C:` with debug symbols and incremental compilation disabled; protected compiler temp files were on `D:`. The Cargo feature was enabled because existing unit tests in this worktree require the `sync-s3` test-only File Gate API.

1. Path/reparse/race test:

```text
cargo +1.96.0 test --locked --offline --features sync-s3 -p aquilum-core files::gate::path_guard::tests::ordinary_filegate_write_rejects_escape_and_pins_path_against_replacement -- --exact --nocapture --test-threads=1
```

Result: `1 passed; 0 failed`. It checked a successful in-root save and copy, blocked a rename of a pinned parent while the save was paused, rejected an ancestor junction for create/copy/folder-tree creation, rejected an unregistered-root folder, and kept the outside sentinel unchanged. Runtime marker:

```text
PC_FG_04_PATH_GUARD PASS note_hash=45df22c72c6e776f178bdbb8103c5cac32795afaf8d736fb12c848264ae87d11 outside_hash=d941c06189c8076d17e95a3477d9d32c4e56c3b91ffe2fae65e5972f18e147f7 replacement_blocked=true junction_denied=true copy_junction_denied=true folder_junction_denied=true folder_outside_denied=true outside_root_denied=true
```

2. Folder-tree File Gate regression test:

```text
cargo +1.96.0 test --locked --offline --features sync-s3 -p aquilum-core files::gate::write::folder_tests::core_file_gate_serializes_folder_creation_without_clobber -- --exact --nocapture --test-threads=1
```

Result: `1 passed; 0 failed`.

3. Library compile:

```text
cargo +1.96.0 check --locked --offline -p aquilum-core --lib
```

Result: passed; the compiler emitted existing warnings.

4. `git diff --check` and `rustfmt +1.97.1 --check --edition 2021` for `path_guard.rs` and `entries.rs`: passed.

## Evidence files and SHA-256

Logs are under `D:\mobakula\.artifacts\one-pc-checks\PC-FG-04-20261024\`:

| File | SHA-256 |
|---|---|
| `path-guard-test-nodebug.stdout.log` | `E7E3146BE89EC272D25294D31F27FE83963B5A7CC04CDBB7F099A52E81C84656` |
| `path-guard-test-nodebug.stderr.log` | `ACE9865CCF121F004EC8F0BBCBCF29AFB81BC28120EB3B4FBB74E368F0660316` |
| `folder-tree-test.stdout.log` | `E774A08751A56D91BCF44795962238B623DDDFB41260E7EC1B8E54F082546768` |
| `folder-tree-test.stderr.log` | `5386ED5EADD027F0787A33EC73BD4CE9BD3D01DE7F9D6F5FCDBC298063986B58` |
| `check-lib-after-edits.stderr.log` | `D11B407E3568204AE0445CB9D3C7E9553A444A0ABE9581D10D3334C1E4F9543E` |

SHA-256 of the local source files at report time:

| File | SHA-256 |
|---|---|
| `aquilum-app/core/src/files/gate/path_guard.rs` | `9853DF3C8C5B793F862CF1795CAAD95CCF34DE51B7FC545B22498E70F979BC2C` |
| `aquilum-app/core/src/files/entries.rs` | `6E88195527C574FA15010F68681DF7BD72B1DDCA47A268757733112867AC5673` |
| `aquilum-app/core/src/files/gate/write.rs` | `5693557559DDD6ECC500B8C292662C4417558775C6B5F2F081041FC4B2EB083F` |
| `aquilum-app/core/src/files/document.rs` | `4F8B3945FA62D5A86DBBADD05E29F789442001BA219560C0F0320DC68F67D9EC` |

## Limits

- Windows admission currently supports local drive-letter paths and fails closed for UNC workspace paths. Non-Windows builds perform link checks but do not hold race-resistant directory handles.
- The tests prove parent-directory replacement is blocked during an ordinary File Gate save. They do not prove hostile same-user leaf-inode compare-and-swap: `AtomicWriteFile` still publishes by pathname. Rename/trash/retention routes are outside this report's ordinary write/create/copy/folder coverage and need their own path audit before claiming every File Gate mutation is confined.
- No PR was opened and no Notion status was changed. This is local implementation and evidence only.

## Current local revalidation — 2026-10-24

This section supersedes the earlier source hashes and the earlier limit saying rename/trash/retention were unaudited. The checkout was already dirty when this work began; its other local changes were preserved. Branch and commit remain `main` / `530fe3d6794ee28d2c7098af1266ff5340903543`.

### Additional route coverage

- Rename and move now require both endpoints to resolve to the same registered workspace. Two unregistered paths are denied.
- Link rewrites acquire path leases before reads and writes; rollback uses the same admission callback.
- Trash and restore admit the selected workspace and every source/destination path. Recursive relocation now requests a guard immediately before each path mutation, including newly chosen collision paths and parent creation.
- Trash creation, retention deletion, and full empty-trash purge use registered-workspace path guards. External sources are refused before creating `.trash`.
- Windows directory guards include the selected directory itself where required. The tests block parent replacement during rename and trash, and reject junction ancestors, unregistered rename, outside trash, outside retention, and outside empty-trash.

### Final verification

All commands ran locally with Rust/Cargo 1.97.1, `--locked --offline`, and the `sync-s3` feature.

```text
cargo +1.97.1 check --manifest-path D:\mobakula\aquilum-app\Cargo.toml --locked --offline --features sync-s3 -p aquilum-core --lib
```

Result: passed (`aquilum-core` library checked; existing compiler warnings remain).

```text
cargo +1.97.1 test --manifest-path D:\mobakula\aquilum-app\Cargo.toml --locked --offline --features sync-s3 -p aquilum-core files::gate:: -- --nocapture --test-threads=1
```

Result: 35 passed, 0 failed.

```text
cargo +1.97.1 test --manifest-path D:\mobakula\aquilum-app\Cargo.toml --locked --offline --features sync-s3 -p aquilum-core files::gate::path_guard::tests::ordinary_filegate_write_rejects_escape_and_pins_path_against_replacement -- --exact --nocapture --test-threads=1
```

Result: 1 passed. Marker: `PC_FG_04_PATH_GUARD PASS`, including `rename_parent_replace_blocked`, `trash_parent_replace_blocked`, `unregistered_rename_denied`, `outside_trash_denied`, `junction_trash_denied`, `unregistered_retention_denied`, and `unregistered_empty_trash_denied`, all `true`.

`git diff --check` passed. `rustfmt +1.97.1 --check --edition 2021 aquilum-app/core/src/files/gate/path_guard.rs` passed. A check across all touched files still reports formatting differences in those shared dirty files; they were not reformatted wholesale to avoid rewriting unrelated local changes.

### Final logs and SHA-256

Logs are under `D:\mobakula\.artifacts\one-pc-checks\PC-FG-04-20261024-current\`:

| Log | SHA-256 |
|---|---|
| `check-with-empty-trash.log` | `3CBB9838577F52223AC4668CCA743CBF9A293E4CE9E36A20A905DBD8A11619AD` |
| `file-gate-tests-with-empty-trash.log` | `D8BD949EBB8A1CAB17549E3D24112C9AADDEFCD886066B461AE14F229CD86464` |
| `path-guard-with-empty-trash-final.log` | `88C4C6AF7D8CE30A44A47F0A4086FEB584039FC045BE4324D5E4B276DF3F0140` |

SHA-256 of the final local source files:

| File | SHA-256 |
|---|---|
| `aquilum-app/core/src/files/gate/path_guard.rs` | `0C285517AEA3048666F8A13392466388FFE88BA936A8D5C1D7D0DB67A8C335FA` |
| `aquilum-app/core/src/files/gate/moves.rs` | `50A2394326AB7ACE443C57F6BECE7A532F2676665C7A01641BD88178834F6328` |
| `aquilum-app/core/src/files/gate/trash.rs` | `B1305C8175A002D41F8D9BAEB029429AAEA00E8A61262216630503B91BEFA4F8` |
| `aquilum-app/core/src/files/rename.rs` | `4F2BAA5083CF00212FB67D9D2429E51436B6358219AE6398C4024D0906932449` |
| `aquilum-app/core/src/files/relocate.rs` | `9357321AE27401647E8D35043EE1EA0637170415CA134FB08D6E80986D8C2498` |
| `aquilum-app/core/src/files/trash.rs` | `8750317518DE10EC8B6016A0676AF17E3EC2271A690AFAC960056F188D0662DC` |

### Remaining limits and external acceptance

- Windows path admission still supports local drive-letter paths and fails closed for UNC paths. Non-Windows checks do not hold race-resistant directory handles.
- Same-user replacement of a target leaf between pathname checks and the atomic publisher/remover is not proven impossible; this does not claim a leaf-inode compare-and-swap guarantee.
- PR-only GitHub checks, Main PR gate, PR creation, and Notion updates were not run or changed. This records local implementation and evidence, not external acceptance.
- While freeing space for the Rust test build, one generated Cargo test PDB (`watcher_to_search-…pdb`) was removed; Cargo can regenerate it. PDB/RLIB files under the Cargo target were otherwise NTFS-compressed without changing source files.

## Повторная проверка на текущем worktree — 2026-10-24

Текущие локальные исходники повторно проверены на Windows ПК. Branch/HEAD: `main` / `530fe3d6794ee28d2c7098af1266ff5340903543`; дерево по-прежнему содержит незакоммиченные изменения.

```text
cargo test --locked --offline --features sync-s3 -p aquilum-core files::gate:: -- --nocapture --test-threads=1
```

Результат: **35 успешных тестовых исходов, 0 failed**: основной File Gate harness — 33 passed; оба запуска child-process entrypoints — по 1 passed. Включённый junction/race тест вывел `PC_FG_04_PATH_GUARD PASS`: `replacement_blocked`, блокировка rename/trash parent replacement, `junction_denied`, отказ внешней записи/rename/trash/retention — все `true`. Sentinel BLAKE3: note `45df22c72c6e776f178bdbb8103c5cac32795afaf8d736fb12c848264ae87d11`, outside `d941c06189c8076d17e95a3477d9d32c4e56c3b91ffe2fae65e5972f18e147f7`.

Log: `.artifacts/one-pc-checks/PC-FG-04-20261024-current/file-gate-recheck-20261024.log`.

Повторно выполнены `cargo check --locked --offline --features sync-s3 -p aquilum-core --lib` (**exit 0**, 50 warnings), `rustfmt --check --edition 2021 core/src/files/gate/path_guard.rs` и `git diff --check` (**PASS**). Обязательные PR-only Windows workflow и Main PR gate не запускались: для PC-FG-04 PR не существует и создание PR не разрешалось; внешний CI остаётся непроверенным.

## Публикация доказательств — 24.10.2026

Локальный отчёт опубликован в GitHub в отдельной evidence-ветке `codex/pc-fg-04-path-reparse-evidence-20261024`. Ветка содержит только этот отчёт; исходники File Gate из текущего смешанного незакоммиченного worktree в неё не включались. Для PC-FG-04 отдельный PR не создавался; обязательные exact-head CI и Main PR gate остаются непроверенными. Результаты и ссылка на GitHub внесены в [карточку PC-FG-04 в Notion](https://app.notion.com/p/40332a84beec81b3ba37ee0e623e4431).

Сверка исходной базы после публикации показала, что локальный `main` HEAD `530fe3d6794ee28d2c7098af1266ff5340903543` отстаёт на 39 коммитов от текущего GitHub `main` `ef87aa1b64a4be876ebce774780fe23bf2afda3a`. Поэтому source changes из смешанного worktree не переносились в GitHub; сначала требуется изолировать и согласовать PC-FG-04-патч с актуальным `main`.

