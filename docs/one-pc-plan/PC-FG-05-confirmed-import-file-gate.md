# PC-FG-05 — подтверждённый импорт через единый File Gate

Дата: 2026-10-24  
План: [AKULA-ONE-PC-PLAN-01](https://app.notion.com/p/40332a84beec8199b005f7a239210763)  
Задача: [PC-FG-05](https://app.notion.com/p/40332a84beec816fbda0fc040e7d1d40)  
Репозиторий: `D:\mobakula`, ветка `main`  
Локальный HEAD: `530fe3d6794ee28d2c7098af1266ff5340903543`

## Объём и результат

Локальная реализация маршрутов записи в workspace использует существующий Core File Gate. Для операций импорта добавлен явный маркер подтверждения действия пользователя. Отдельный P2P-метод принимает только проверенную staged-запись и публикует её в зарегистрированный workspace через `gate::create_binary`.

Это локальная реализация и проверка кода, не подтверждение установленного P2P UI, GitHub Actions или Main PR gate. Notion, PR и удалённые ветки не изменялись.

## Изменения

- В `aquilum-app/core/src/files/gate/mod.rs` добавлен `ImportConfirmation`. Это маркер API-соглашения, а не capability безопасности. Его следует создавать только после явного действия пользователя; отменённый импорт не вызывает importer.
- `aquilum-app/native/src/vault_files.rs` требует маркер для импорта. Создание папки, копирование файла и сохранение байтов идут через `ensure_folder_tree`, `gate::copy` и `gate::create_binary`.
- Пользовательские вызовы из выбора книги/PDF/изображения, вставки и drag-and-drop передают маркер после соответствующего действия.
- `aquilum-app/core/src/p2p_inbox_lab/file_gate.rs` проверяет открытый regular-file stage, размер до 8 MiB и BLAKE3 перед `gate::create_binary`. Запись создаётся только по новому пути; повтор не заменяет существующий файл.
- Маршрут возвращает ошибку File Gate при попытке записать вне зарегистрированного workspace. Он не объявляет приём P2P-пакета, атомарную публикацию или durable ACK.

## Аудит остальных путей workspace

| Область | Наблюдаемый путь записи |
|---|---|
| Canvas | `core/src/files/structured.rs`: сохранение идёт через `gate::write`. |
| Sync | Guarded note/history writers используют File Gate; sync-журналы и staging в app-data являются отдельным внутренним состоянием. |
| Plugins | API чтения/разрешений не предоставляет произвольную запись в Vault. MCP-инструменты для заметок используют `gate::create` и `gate::write`. |
| Локальные вложения и PDF | Создание вложений/заметок идёт через File Gate после действия пользователя. |
| Сетевой импорт в workspace | Новый Core helper проверяет stage и вызывает `gate::create_binary`; установленный receiver UI в текущем локальном `main` не подключён. |

`save_verified_stage_as_new` остаётся отдельным helper для сохранения новой внешней копии, не для импорта в Vault. В текущем `main` у него нет вызова из UI; сохранённые unit-тесты проверяют его no-replace поведение. Draft PR #694 также ограничивает Save Copy внешним путём и отказывает для известных Vault. Это не является доказательством принятого пользовательского P2P потока в `main`.

## Проверки

Успешный Core сценарий выполнен на временном синтетическом workspace:

```powershell
cargo +1.96.0 test --manifest-path aquilum-app/Cargo.toml --locked --offline --features sync-s3,p2p-inbox-lab -p aquilum-core p2p_inbox_lab:: -- --nocapture --test-threads=1
```

Результат: **6 passed, 0 failed**. Проверены подтверждённое создание, отказ для незарегистрированного пути без изменения файла, повтор без перезаписи, неверный BLAKE3/размер, нулевой и превышающий лимит manifest. Журнал: `.artifacts/one-pc-checks/PC-FG-05-20261024/core-p2p-inbox-tests.stdout.log` и `.stderr.log`; код завершения `0`.

В первоначальной проверке Native test-код, включая тесты локального importer, прошёл compile-check:

```powershell
cargo +1.96.0 check --manifest-path aquilum-app/Cargo.toml --locked --offline -p aquilum-native --bin aquilum-native --tests
```

Результат: **exit 0**, компилятор сообщил предупреждения. На момент той проверки это была только компиляция; позднее runtime-повтор выполнен, результаты приведены ниже. Две первоначальные попытки на `D:` остановились до исполнения из-за нехватки места при компиляции `tantivy`; логи сохранены в `.artifacts/one-pc-checks/PC-FG-05-20261024/native-import-tests*.stderr.log`. Ограниченная попытка на `C:` была вручную остановлена до запуска тестов при 755 MiB свободного места; её частичные target-артефакты сохранены.

`git diff --check` и targeted `rustfmt +1.96.0 --check --edition 2024 --config skip_children=true` для затронутых файлов: **PASS**.

Синтетические байты из теста `confirmed P2P import` имеют SHA-256 `30FD8BF471E3491F95E2F7EA82D7BAF2A2DA9F0B02C7528DB91BEE76C6B27510`; данные теста отказа `outside attempt` — `45036291EBC7850DCC8FC80A3939ABDEE74FFB01024CEB8B9E5EE5C6BA9DD5E8`. Тест проверяет BLAKE3 ожидаемых и записанных байтов.

SHA-256 журналов и команд из `.artifacts/one-pc-checks/PC-FG-05-20261024/`:

| Файл | SHA-256 |
|---|---|
| `core-p2p-inbox-tests.cmd` | `F81E7A2B1B212E65552E38062FABB9FB1FCDE75A53C96456FFE7907504C8B2BA` |
| `core-p2p-inbox-tests.stdout.log` | `9732285FC124C73EF7257F42169D15A6A0FB01FE85F5B7D77BEA33FA80119A1F` |
| `core-p2p-inbox-tests.stderr.log` | `5518849BB44829715435F96A94029E19F20231CB94C3EC456000846A9709A22E` |
| `native-test-check.stderr.log` | `508F97E3B5C5F24B77B73248F08654285D09E182C011C71E186B1500C019587D` |
| `native-import-tests-default.cmd` | `4CF54BB12DD592DCD8A669C49C8FC0124785B11CD3063A7CD08BEFAA56B805C9` |
| `native-import-tests-default.stderr.log` | `87E279BF9D4E9BA67C31B0252BF634FCB565933ED012CAC7D0835522260032E5` |
| `native-import-tests-c-target.cmd` | `CF319296F2450B331E4F00433CDB899716262D95B5E9FB213FDD06160E64BA29` |
| `native-import-tests-c-target.log` | `7C59F160DD6690873D54E51934BB586BD71CFA95E59A90238C972A9AC0BAE4EF` |
| `native-import-tests-c-target.status.txt` | `BAC66206AF8FFEF695FC7AF91A96BAB41DE19ACDB859A43187D6692AF5562916` |

## GitHub и границы приёмки

- Локальный `HEAD` — `530fe3d6794ee28d2c7098af1266ff5340903543`; GitHub `main` — `0eb59af2b50aea1c262d6afba9832196b6ea9276`, его родитель — локальный HEAD.
- PR #614 остаётся Draft на старой базе (`d4487a78083150924a7a8b8ea85693d931041960`) и содержит близкий Gate-маршрут. Локальная версия добавляет обязательный маркер подтверждения в API.
- PR #676 остаётся Draft: Canvas Core с Gate-backed save есть, пользовательский UI ещё не подключён.
- PR #691 остаётся Draft: receiver использует отдельный Received Inbox; Vault import не является принятой функцией.
- PR #694 остаётся Draft на базе локального HEAD (`530fe3d`), актуальный GitHub head при повторной сверке — `7d5138facb3acbddb236278c985dbaf1b7d6ce5a`. В актуальном описании подтверждены установленный P2P интерфейс и отсутствие Vault/File Gate apply. Доступная локальная remote-tracking ссылка отстаёт (`1d1c82c1a8e4d8bd5105834cb2624c49c219c241`); в её `native/src/app/transfer/inbox.rs` выбранное действие явно является внешним `Save Copy` и вызывает `save_verified_stage_as_new`, а не Gate-маршрут. Эту старую ссылку нельзя считать точным состоянием PR.
- Повторная проверка локальной среды: на `C:` свободно 0,74 GiB, на `D:` — 0,55 GiB; процессов `cargo`/`rustc` нет. Для Native importer runtime-тестов свободного места по-прежнему недостаточно без риска остановки сборки на почти полном диске.
- Workflow-файлы `sync-guarded-transaction.yml` и `sync-pinned-read.yml` проверены. Для этого локального, незапушенного изменения удалённый прогон не подтверждает код; PR, push и dispatch не выполнялись.
- Карточка PC-FG-05 в Notion по-прежнему имеет статус «В очереди»; это намеренно, поскольку пользователь запретил менять Notion.

## Осталось

- Native importer runtime-тесты повторно выполнены после освобождения только старого Cargo build cache; актуальные результаты приведены ниже.
- Не подключён установленный сетевой Inbox UI к явному Gate import; текущая main-ветка не содержит принятого receiver UI.
- Main PR gate и связанные GitHub Actions для точного изменённого HEAD не подтверждены. Для этого потребуется опубликованный точный HEAD, что сейчас запрещено инструкцией пользователя.
- Следующая задача по зависимости: PC-FG-06 — негативные сценарии; выполнять после учёта оставшихся проверок PC-FG-05.

## Повторная локальная runtime-проверка — 24.10.2026, 13:57–14:10 +05

Повтор выполнен после поздних изменений File Gate path guard. Входной/итоговый HEAD `530fe3d6794ee28d2c7098af1266ff5340903543`, ветка `main`, Cargo.lock SHA-256 `33103743A49376BFAA14C9BEE40389A03AD2CF7EDE31158537440A64FF012319`; Rust `1.96.0-x86_64-pc-windows-msvc`. Оба запуска были `--locked --offline`, один build job, incremental compilation и debug info выключены для экономии диска; `TEMP`, `TMP` и Cargo target находились под `D:\mobakula\.artifacts`.

- Native пользовательские локальные импорты: `cargo +1.96.0 test --manifest-path aquilum-app/Cargo.toml --locked --offline -p aquilum-native --bin aquilum-native vault_files::tests::attachment_import -- --nocapture` — **2 passed, 0 failed** (750 filtered). Run `.artifacts/one-pc-checks/PC-FG-05-20261024-native-runtime-retry2/summary.json`. Один лог `file is not a database` ожидаем: тест намеренно портит синтетический participant registry и проверяет fail-closed поведение.
- Core подтверждённый импорт из проверенного stage: `cargo +1.96.0 test --manifest-path aquilum-app/Cargo.toml --locked --offline --features sync-s3,p2p-inbox-lab -p aquilum-core --lib p2p_inbox_lab:: -- --nocapture --test-threads=1` — **6 passed, 0 failed** (1460 filtered). Run `.artifacts/one-pc-checks/PC-FG-05-20261024-core-runtime-retry/summary.json`. Проверены запись через зарегистрированный File Gate, отказ вне workspace без изменения данных, повтор без перезаписи и отказ на неверном digest/размере.
- Первая попытка Native с `--lib` завершилась сообщением Cargo `no library targets found`; тесты не запускались. Исправленный вызов использовал бинарную цель `--bin aquilum-native` и прошёл.
- Для освобождения места сохранён EXE PC-00.02 по исходному пути; его SHA-256 после очистки совпадает с отчётом: `3A623CA29DC76BAB738205A399979A9F6236944FC9AFD27C3EC6B6EF9BB65EFC`. Удалены только остальные генерируемые файлы старого `.artifacts/cargo-pc-00-02/debug` target (около 4,34 GiB); исходники, журналы, профили и фикстуры не затронуты.
- `git diff --check` — PASS. Код, Cargo.lock, Git refs, PR и Notion не менялись.

Хеши актуальных runtime-журналов:

| Артефакт | SHA-256 |
|---|---|
| Native `stdout.log` | `76B511CA322A0980A1982E911607D5C3E06B37D92DB7C8BB0C8059CC64B06566` |
| Native `stderr.log` | `B505A039101A06D1C986909186006B56571F4FFC0F3CC3FC4410D4EE1517ACA1` |
| Native `summary.json` | `D3BB1E67E69F435396347B2319C14D2757AF0D62AB8D7FFF30F4FCBB57543E7A` |
| Core `stdout.log` | `7A5CCB6133D9BBD647EDF45E3053CB0A304EBB228ED1AD101015FA54FDAD5983` |
| Core `stderr.log` | `2D0AA0503BB12CF89A97A392DA052C9705666544275B21DCAE82688BD026D219` |
| Core `summary.json` | `47F434423910A6B1179056D07B486432F1DD8A688833AC8742A1691AB7BEC128` |

**Граница результата:** Core Gate-маршрут и связанные локальные import-тесты подтверждены. Установленный сетевой Inbox UI отсутствует в локальном `main`; открытый Draft #694 сохраняет отдельный Save Copy маршрут и не является установленным Gate-backed импортом в Vault. Результат не заявляет P2P end-to-end или GitHub Main PR gate. Продолжение P2P пользовательского пути должно переиспользовать существующую работу, не создавать второй транспорт и учитывать запрет на изменение PR.



---

## Publication update — 2026-10-24

This report has been published in the documentation-only one-PC evidence PR [#699](https://github.com/Lalalalendia/Akula/pull/699). The report records local evidence and its stated limits; this documentation PR does not include the underlying application source changes. PC-FG-01 implementation remains in Draft [#703](https://github.com/Lalalalendia/Akula/pull/703); PC-FG-02 test code/report are in Draft [#704](https://github.com/Lalalalendia/Akula/pull/704). No PR was merged.
