# PC-FG-06 — негативные испытания File Gate

- Дата: 2026-10-24
- План: [AKULA-ONE-PC-PLAN-01](https://app.notion.com/p/40332a84beec8199b005f7a239210763)
- Карточка: [PC-FG-06](https://app.notion.com/p/40332a84beec8146ae28e166b50b9cb1)
- Репозиторий: `D:\mobakula`, ветка `main`
- HEAD до и после работы: `530fe3d6794ee28d2c7098af1266ff5340903543`

## Критерии

- Недостаток места, запрет записи, повтор, отказ после подтверждения, устаревшая ревизия, выходящий за Vault путь и неверный хеш не должны возвращать ложный успех.
- После отказа исходные/уже сохранённые байты остаются проверяемыми; неполный файл, который не удалось убрать, требует ручной проверки перед повтором.
- Проверки выполняются на изолированных синтетических каталогах. Недостаток места моделируется ошибкой записи, без заполнения системного диска.

## Изменения

- `aquilum-app/core/src/files/entries.rs`: если `write_all`/`sync_all` завершились ошибкой, File Gate пытается удалить незавершённый новый файл. Если удаление не удалось, возвращается `NeedsReview` с указанием неполного пути и запретом автоматического повтора.
- Там же добавлены детерминированные тесты частичной записи с `StorageFull` и отказа очистки с `PermissionDenied`. Первый требует отсутствия частичного файла; второй требует `NeedsReview` и сохранённые фактические байты для проверки.
- `aquilum-app/core/src/files/gate/tests.rs`: добавлены тесты stale-version через публичный File Gate и Windows read-only файла. Они требуют ошибки и сохранения внешней/исходной ревизии.
- `aquilum-app/core/src/files/gate/write.rs`: исправлена фикстура существующего теста двух Core — оба экземпляра теперь регистрируют общий тестовый Vault до `ensure_folder_tree`.
- `aquilum-app/core/src/p2p_inbox_lab/file_gate.rs`: запрещённый destination теперь проверяет traversal `vault/../outside`; подтверждённый импорт не должен создать файл за корнем.

## Проверки на Windows

Все команды запущены на одном ПК. Cargo использовал `CARGO_TARGET_DIR=C:\Users\User\.codex\visualizations\2026\10\22\01a164fd-ca9a-7513-92bf-2a106f50393e\cargo-pc-fg04-nodebug-20261024`, `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_PROFILE_DEV_DEBUG=0`.

```powershell
cargo +1.96.0 test --manifest-path aquilum-app/Cargo.toml --locked --offline --features sync-s3,p2p-inbox-lab -p aquilum-core --lib files:: -- --nocapture --test-threads=1
# 117 passed, 0 failed, 1 ignored

cargo +1.96.0 test --manifest-path aquilum-app/Cargo.toml --locked --offline --features sync-s3,p2p-inbox-lab -p aquilum-core --lib p2p_inbox_lab:: -- --nocapture --test-threads=1
# 6 passed, 0 failed
```

В `files::` вошли simulated StorageFull cleanup, реальный Windows read-only отказ, stale Gate version и path-guard traversal/replacement сценарии. В `p2p_inbox_lab::` вошли hash отказ, повтор без перезаписи и подтверждённый импорт в зарегистрированный Vault. `rustfmt +1.96.0 --check` для затронутых Rust-файлов и `git diff --check` прошли.

Первая попытка набора завершилась 115 passed и 2 failed: тест очистки ошибочно предполагал, что Windows не удаляет файл с атрибутом readonly; ещё одна существующая фикстура двух Core не регистрировала Vault. Оба тестовых дефекта исправлены. Последний прогон зелёный: 117 + 6. Полные журналы и точные повторяемые команды находятся в `.artifacts/one-pc-checks/PC-FG-06-20261024/`; SHA-256 последнего прогона:

| Артефакт | SHA-256 |
|---|---|
| `core-files-attempt3.log` | `BE9738939A65EDE05F08142F4851F79DB6BBAF0A9EA8B383F4DE4EBFC2A316CB` |
| `core-p2p-inbox-attempt3.log` | `BD4CC2F42F6478DE7D6FC9B76202226373FE2C1063EA33D086D8D34A1D96F239` |
| `status-attempt3.txt` | `A375F7172485438A02FCB850748BC2C4DB03EC06CBB2AAE2C564698C0CD0B18D` |
| `run-attempt3.ps1` | `077CBFCDB8026AE82D7469CFE9DFDC1572FE2F92FA7CC6D9F1837E84314FFA10` |

SHA-256 тестовых UTF-8 данных: `partial candidate` — `E2BC4B58F38E2FC1F1A9255B78E7D275C63C8CF41DA603BA825CAD630C33D3C4`; `original bytes` — `52C3935626C104B2CBC9031291A1C4D56614C38F52072A361D658A58A9C48698`; `external revision` — `1754C1FA59AEA62CB3857520F184AA6368B16094D9C45D75FC27A0D47D53C77B`; `confirmed P2P import` — `30FD8BF471E3491F95E2F7EA82D7BAF2A2DA9F0B02C7528DB91BEE76C6B27510`; `outside attempt` — `45036291EBC7850DCC8FC80A3939ABDEE74FFB01024CEB8B9E5EE5C6BA9DD5E8`; `actual bytes` — `13D88C6C0D70BBFF19ED19F9B545ED2919687882F04A10B02BA627CF0144449A`; `forged bytes` — `8A44877D7C0EEA3CE87F9FC670E23DC48D8FE1778F4150788AFA31780A817590`.

## Ограничения и следующий шаг

- StorageFull — fault injection; реальный диск намеренно не заполнялся. В прежнем запуске на системных дисках оставалось меньше 1 GiB; актуальные перепроверки использовали локальный target на `D:` и не занимали диск искусственно.
- Native importer runtime-тесты из PC-FG-05 повторно прошли 2/2 на текущем локальном HEAD. P2P Inbox UI → Vault File Gate apply всё ещё отсутствует в локальном `main`; PR #694 остаётся Draft и описывает отдельный Received Inbox/Save Copy без Vault/File Gate apply.
- GitHub Main PR gate и статус/доказательство в Notion не обновлялись: push, PR update/merge и запись в Notion запрещены текущей инструкцией.
- Локальная проверка не является проверкой на другом компьютере или внешней сети.

Следующая карточка по порядку пакета A в разделе 13 плана: **PC-DESK-01 [Л]** — офлайн-открытие, редактирование, сохранение, повторное открытие и проверка истории/отката Markdown по содержимому и SHA.

## Повторная проверка текущего кода — 24.10.2026, 14:14–14:16 +05

После изменений File Gate/path guard старые прогоны не использовались как актуальное подтверждение. На текущей рабочей копии повторены связанные наборы с Rust `1.96.0-x86_64-pc-windows-msvc`, `--locked --offline`, `CARGO_TARGET_DIR=D:\\mobakula\\.artifacts\\cargo-native`, `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_PROFILE_DEV_DEBUG=0`; временные файлы оставались внутри workspace. Debug info отключён только для экономии места; исходники и Cargo.lock не менялись.

| Проверка | Результат | Журнал |
|---|---|---|
| Core `files::` | **117 passed, 0 failed, 1 ignored** | `.artifacts/one-pc-checks/PC-FG-06-20261024-current-recheck/core-files.stdout.log` |
| Core `p2p_inbox_lab::` | **6 passed, 0 failed** | `.artifacts/one-pc-checks/PC-FG-06-20261024-current-recheck/core-inbox.stdout.log` |
| Targeted rustfmt (`aquilum-core` edition 2021) | **PASS** для `entries.rs`, `gate/tests.rs`, `gate/write.rs`, `gate/path_guard.rs`, `p2p_inbox_lab/file_gate.rs` | `rustfmt-2021.command.txt` |
| `git diff --check` | **PASS** | итоговый локальный diff |

Запуски охватили injected StorageFull/cleanup, PermissionDenied/read-only, stale Gate version, Windows path guard traversal/replacement, подтверждённую запись в зарегистрированный workspace, отказ вне workspace, неверный digest/размер и повтор без перезаписи. StorageFull остаётся fault injection, физический диск не заполнялся. Обе команды завершились с кодом 0 на `HEAD=530fe3d6794ee28d2c7098af1266ff5340903543`; Cargo.lock SHA-256 — `33103743A49376BFAA14C9BEE40389A03AD2CF7EDE31158537440A64FF012319`.

`rustfmt` сверялся с edition 2021 из `aquilum-app/core/Cargo.toml`. Первоначальная диагностическая команда с принудительным edition 2024 показывала только другой порядок `use` и не является критерием приёмки; корректный edition 2021 check прошёл без правки исходников.

Текущий GitHub поиск `PC-FG-06` не нашёл отдельного PR. PR [#614](https://github.com/Lalalalendia/Akula/pull/614) остаётся Draft для отдельного Core P2P Gate route; PR [#694](https://github.com/Lalalalendia/Akula/pull/694) остаётся Draft и не подключает Vault/File Gate apply. Ни один PR, workflow, Git ref или Notion page не менялся. Это локальные проверки на одном ПК; Main PR gate, установленная P2P приёмка и внешний сетевой тест не заявляются.

SHA-256 актуальных журналов: `core-files.stdout.log` — `426FFB84502ED0B261CE76364EBC95AA2FB4BBD49CCDB626C2B87056EC89F954`; `core-files.stderr.log` — `933D079B7C079F718D9537568E55A4F5B0ED17A30C7FB34942F9322291E2DA78`; `core-inbox.stdout.log` — `7A5CCB6133D9BBD647EDF45E3053CB0A304EBB228ED1AD101015FA54FDAD5983`; `core-inbox.stderr.log` — `58EB07A0242E2CE4532E43F69F9ECE3FD07E82997EC3A220CD26570FD8B73183`; итоговый `summary.json` — `0657702C3E0252283C75594B1ED85783D39B809E92189CF842704D5A9687E7F8`.



---

## Publication update — 2026-10-24

This report has been published in the documentation-only one-PC evidence PR [#699](https://github.com/Lalalalendia/Akula/pull/699). The report records local evidence and its stated limits; this documentation PR does not include the underlying application source changes. PC-FG-01 implementation remains in Draft [#703](https://github.com/Lalalalendia/Akula/pull/703); PC-FG-02 test code/report are in Draft [#704](https://github.com/Lalalalendia/Akula/pull/704). No PR was merged.
