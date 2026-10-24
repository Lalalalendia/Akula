# PC-FG-01 — межпроцессная блокировка обычных записей File Gate

Локальное выполнение плана `AKULA-ONE-PC-PLAN-01`, 2026-10-24. Отчёт фиксирует проверяемые результаты на одном физическом ПК. Состояние публикации и CI актуализировано ниже.

## Объём

- Обычный desktop Akula; мобильный сценарий не затрагивался.
- Два отдельных процесса Akula с независимыми app-data работают с одним и тем же локальным файлом.
- Второй процесс не может менять файл, пока первый удерживает File Gate между публикацией основных байтов и фиксацией истории.

## Реализация

- `Core.file_mutations` теперь объединяет короткий mutex процесса с OS file lock, общим для процессов того же пользователя Windows или Unix. Имя lock-файла выводится из пользовательского профиля и находится в системном temp-каталоге.
- Lock-файл не удаляется после использования: удаление открытого lock-файла могло бы позволить другому процессу захватить новый объект под тем же именем.
- После захвата код повторно открывает имя lock-файла и сверяет идентичность открытого объекта. На Unix проверяются обычный файл, режим доступа, число ссылок и отсутствие symlink; на Windows используются `OPEN_REPARSE_POINT`, запрет совместного удаления и идентичность по открытому Win32 handle.
- Порядок захвата: локальный mutex → общий OS lock. В `write`, `create` и строгой истории локальный mutex освобождается после записи основного файла, а OS lock остаётся удержанным до завершения истории. Это сохраняет общий процессный путь свободным на время SQLite, не открывая межпроцессное окно между файлом и историей.
- Существующие операции, уже проходящие через общий `Core.file_mutations`, автоматически используют тот же OS lock.
- Это кооперативная блокировка процессов одного пользователя, а не граница авторизации против вредоносного процесса с теми же правами. Она глобальна для процессов пользователя и потому может сериализовать записи в разные workspace.

## Проверки на одном ПК

Тест двух процессов создаёт изолированные app-data A и B и общий синтетический workspace. Процесс A записывает файл и останавливается после публикации байтов, до истории; в это время процесс B запускается с хешем версии A и не может изменить файл. После продолжения A оба процесса завершают запись последовательно. Затем тест повторяет запись с исходным устаревшим хешем и проверяет отказ без изменения байтов B.

Проверенные BLAKE3:

| Состояние | BLAKE3 |
| --- | --- |
| Начальный файл до конкурирующих записей | `6a8b07a8bf00bf688b0962d3ae37c9f41744da1e718c52d3b39f54bf087d2d2f` |
| После записи процесса A | `a206fff60e82dd6698d71d954e096bc27ab44d7ef0a13a7b59ddb1dc03cda068` |
| После записи процесса B | `9f5811975b5501438e69e03f401c2d3b71b4d7113c1f8e412528e4d18cacfcfa` |
| Повторная запись со stale-хешем | отказ; байты и хеш процесса B остались прежними |

На Windows строгий тест истории использовал выделенный временный каталог с отключённым наследованием ACL и полным доступом только текущего пользователя, SYSTEM и Administrators. Обычный системный TEMP не проходит предусмотренную этим тестом проверку ACL внешней области хранения; повтор с изолированным ACL проходит.

### Результаты

- Двухпроцессный сценарий `two_process_filegate_writes_stay_serialized_through_history`: **PASS**, 1 тест на текущем исходном коде.
- Набор `files::gate::` без дополнительных feature-флагов: **PASS**, 33 теста.
- Строгий тест истории `strict_history_witness_is_exact_new_external_version_under_one_filegate`: **PASS**, 1 тест с выделенным ACL-ограниченным TEMP.
- `git diff --check`: **PASS**.
- `rustup run 1.97.1 rustfmt --edition 2021 --check aquilum-app/core/src/files/gate/interprocess_lock.rs`: **PASS**. Массовое форматирование не выполнялось.

Команды Cargo выполнялись на Rust `1.97.1` с `--locked --offline`, `CARGO_INCREMENTAL=0`, одним build job и существующим target-каталогом `D:\mobakula\.artifacts\cargo-native`. Успешная сборка и тесты используют текущий lockfile и рабочие файлы.

Актуальные журналы успешных прогонов сохранены в `.artifacts/one-pc-checks/PC-FG-01-20261024-current/`:

- `interprocess-d-target.log`
- `filegate-suite-d-target.log`
- `strict-history-restricted-temp.log`

Предыдущие журналы в `PC-FG-01-20261024-final` созданы до последних изменений исходников и для этой приёмки не использовались. Первая свежая сборка в visualization target была остановлена до запуска тестов, когда на C: осталось 0,20 ГиБ; успешный повтор выполнен в существующем target на D:. Никакие build-файлы при этой повторной проверке не удалялись.

## Публикация и границы приёмки

- Создан Draft PR [#703](https://github.com/Lalalalendia/Akula/pull/703) в `Lalalalendia/Akula`; PR не объединён.
- Точный HEAD опубликованной версии: `7dcf1395f7e96ca21bbd78a9d4af00d9168eb52a`, основание — `main` `0eb59af2b50aea1c262d6afba9832196b6ea9276`.
- В PR ровно четыре файла PC-FG-01: `app_core.rs`, `files/gate/interprocess_lock.rs`, `files/gate/mod.rs`, `files/gate/write.rs`. Соседний `interprocess_probe.rs`, P2P, mobile, UI, manifest и lockfile не включены.
- Actions на точном HEAD: [Main PR gate #40089339945](https://github.com/Lalalalendia/Akula/actions/runs/40089339945), [File Gate History #40089339740](https://github.com/Lalalalendia/Akula/actions/runs/40089339740), [Sync single-custody #40089339727](https://github.com/Lalalalendia/Akula/actions/runs/40089339727), [Sync S3-bound custody #40089339828](https://github.com/Lalalalendia/Akula/actions/runs/40089339828), [native two-process acceptance #40089339908](https://github.com/Lalalalendia/Akula/actions/runs/40089339908). На момент обновления они ещё выполнялись.
- Первые CI-ошибки относились к superseded HEAD `eb58d19c7347cc5262e700bddbf6a3865369bce0`; следующий HEAD `9b60b0c90b704ab9ab2cfa0f4f8d2bfe085f1a75` дошёл до проверки пробелов, которая нашла одну пустую строку в конце нового файла. Она удалена в текущем коммите, поэтому красный результат не относится к нынешней версии.
- Notion PC-FG-01 обновлена: статус `Ожидает CI`, сохранены ссылка на PR, SHA и URL проверок.
- Локальные PASS-тесты запускались на полном изменённом checkout, а не на точном опубликованном дереве. Завершение задачи зависит от обязательных Actions на текущем HEAD.
- Приёмка относится к одному физическому ПК и двум процессам; проверка на втором физическом устройстве не заявляется.

## Повторная сверка CI — 24.10.2026, 16:20 +05

Эта запись уточняет состояние выше: [Application checks #40089339945](https://github.com/Lalalalendia/Akula/actions/runs/40089339945) всё ещё выполняет часть jobs, но обязательный job `Required Rust tests and policy checks` уже завершился с ошибкой `Enforce no-growth source and workflow fanout ratchet`. Выявлены три превышения размера исходников: `app_core.rs` 601→690 строк, `files/gate/mod.rs` 122→124, `files/gate/write.rs` 525→544. Architecture budget сообщает +582 production lines; это advisory, не отдельная причина провала.

[File Gate History #40089339740](https://github.com/Lalalalendia/Akula/actions/runs/40089339740) завершился успешно. Native two-process acceptance #40089339908, Sync S3-bound custody #40089339828 и Sync single-custody #40089339727 ещё выполнялись на момент сверки. Notion PC-FG-01 переведена в статус «Нужна проверка»; следующий шаг — переработать объём исходников, не меняя CI-пороги, затем повторить проверки на новом точном HEAD.

По повторному снимку в 16:22 +05 [Sync single-custody #40089339727](https://github.com/Lalalalendia/Akula/actions/runs/40089339727) также завершился успешно. Application checks #40089339945 остаётся в процессе с упавшим обязательным no-growth job; Native two-process acceptance #40089339908 и Sync S3-bound custody #40089339828 ещё выполняются. Состояние синхронизировано с Notion PC-FG-01.

## Локальная повторная приёмка после переноса interlock — 24.10.2026

Рабочая копия была на `main` HEAD `530fe3d6794ee28d2c7098af1266ff5340903543`; дерево уже содержало несохранённые изменения в других задачах, они не сбрасывались. Межпроцессная блокировка, Core interlock и тестовая пауза собраны в `files/gate/interprocess_lock.rs`; `app_core.rs` подключает этот модуль напрямую. Test pause срабатывает после освобождения process mutex, пока OS lock остаётся удержанным до завершения истории. Это уменьшает рост прежних файлов в локальном изменении, не меняя CI-порог.

Проверки на текущем Windows ПК, Rust 1.97.1, `--locked --offline`, `CARGO_TARGET_DIR=D:\\mobakula\\.artifacts\\cargo-native`:

- `cargo test --lib app_core::interprocess_lock::tests:: -- --nocapture`: **2 passed, 0 failed**; отдельные процессы напечатали baseline/A/B BLAKE3 `6a8b07a8…`, `a206fff6…`, `9f581197…`; stale write отклонён.
- `cargo test --lib files::gate:: -- --nocapture`: **31 passed, 0 failed**.
- `rustfmt --edition 2021 --check aquilum-app/core/src/files/gate/interprocess_lock.rs`: **PASS**.
- `git diff --check` по изменённым tracked-файлам PC-FG-01: **PASS**.

GitHub PR #703 остаётся Draft на старом exact HEAD `7dcf1395f7e96ca21bbd78a9d4af00d9168eb52a`; его Required Rust tests and policy checks всё ещё показывают no-growth failure. Локальная переразбивка не опубликована и не проверена GitHub Actions; PR и Notion в этой итерации не менялись.



---

## Publication update — 2026-10-24

This report has been published in the documentation-only one-PC evidence PR [#699](https://github.com/Lalalalendia/Akula/pull/699). The report records local evidence and its stated limits; this documentation PR does not include the underlying application source changes. PC-FG-01 implementation remains in Draft [#703](https://github.com/Lalalalendia/Akula/pull/703); PC-FG-02 test code/report are in Draft [#704](https://github.com/Lalalalendia/Akula/pull/704). No PR was merged.


## Точная сверка CI и локальная повторная приёмка — 24.10.2026

Текущий Draft [PR #703](https://github.com/Lalalalendia/Akula/pull/703) остаётся на exact HEAD `8445864f28123fb026d4881a59c5554f3b5829dc`.

- Профильные Actions на этом HEAD: [File Gate History #40089888724](https://github.com/Lalalalendia/Akula/actions/runs/40089888724), [Sync single-custody #40089888718](https://github.com/Lalalalendia/Akula/actions/runs/40089888718), [Sync S3-bound custody #40089888716](https://github.com/Lalalalendia/Akula/actions/runs/40089888716), [Native two-process #40089888759](https://github.com/Lalalalendia/Akula/actions/runs/40089888759): все четыре завершились успешно.
- [Application checks #40089888780](https://github.com/Lalalalendia/Akula/actions/runs/40089888780) завершились ошибкой fail-closed. Job классификации #120862804201 прошёл 7 тестов dependency-security gate, но затем остановился: watchdog run [#40090177003](https://github.com/Lalalalendia/Akula/actions/runs/40090177003) ещё не завершился. Этот watchdog впоследствии завершился SUCCESS на main SHA `23383c52a6a63fb2e2f6b71d567e41e9018cda22`. Из-за раннего отказа обязательные Rust tests/policy checks были пропущены; no-growth на exact HEAD этим запуском не подтверждён.
- Локально, на Windows и полном изменённом checkout (основание `530fe3d6794ee28d2c7098af1266ff5340903543`): межпроцессные тесты **2 passed**, File Gate suite **31 passed**, rustfmt **PASS**, `git diff --check` **PASS**. Checkout содержал изменения соседних задач; эти результаты не подменяют CI точного дерева PR #703.

**Действие:** повторить Application checks на том же HEAD после успешного watchdog, затем проверить результат Main PR gate/no-growth. PR остаётся Draft; слияние не заявляется.
