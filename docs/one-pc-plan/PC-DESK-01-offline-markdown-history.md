# PC-DESK-01 — офлайн Markdown и восстановление истории

**Notion:** [PC-DESK-01](https://app.notion.com/p/40332a84beec816e8f6dfcaff17b28a8) · [единый план](https://app.notion.com/p/40332a84beec8199b005f7a239210763)

**Проверка:** 24.10.2026, один физический Windows ПК, изолированные временные папки.

**Git:** `main`, исходный и итоговый `HEAD` — `530fe3d6794ee28d2c7098af1266ff5340903543`; локальный `origin/main` совпадает.

## Результат

Добавлен сквозной тест Native-приложения `offline_markdown_edit_restart_and_history_restore_match_exact_hashes`. Он открывает синтетическую Markdown-заметку, редактирует её в редакторе, сохраняет через обычный путь File Gate, закрывает и пересоздаёт приложение на той же временной рабочей папке, затем выбирает сохранённую версию в панели истории и запускает действие Restore.

Проверены точные байты и SHA-256:

| Состояние | SHA-256 |
| --- | --- |
| Сохранённая версия A | `5052fdf0da2e55d6df789b618a475052c84384ba8fb5da9159ec7c9f2aa51b08` |
| После правки и перезапуска | `6f30c14ae1ed4991f0f06cb9d09c57ba4991866db9ff04b0bb83408bfc9abbb7` |
| После Restore версии A | `5052fdf0da2e55d6df789b618a475052c84384ba8fb5da9159ec7c9f2aa51b08` |

Отрицательная проверка: запрос отсутствующего ID истории возвращает `false` и оставляет файл с тем же SHA-256. Тестовые данные — только синтетический русский текст; облачные сервисы и пользовательские файлы не используются. Между контрольными сохранениями второй тестовый текст пересекает порог HistoryService в 200 изменённых символов, чтобы штатное объединение быстрых небольших правок не схлопнуло две контрольные версии.

## Команды и доказательства

```powershell
cargo +1.96.0 test --manifest-path aquilum-app/Cargo.toml --locked --offline -p aquilum-native --bin aquilum-native app::tests::notes::offline_history::offline_markdown_edit_restart_and_history_restore_match_exact_hashes -- --exact --nocapture
rustup run 1.96.0 rustfmt --check --edition 2024 --config skip_children=true aquilum-app/native/src/app/tests/notes/offline_history.rs
git diff --check
```

Результат теста: **1 passed, 0 failed, 752 filtered out**. `rustfmt --check` и `git diff --check` прошли. Полный журнал: `.artifacts/one-pc-checks/PC-DESK-01-20261024-local/native-offline-history.log`; SHA-256 журнала: `4b0c2ba9bb485289cb03b021e2d804ac24f42496cc55fc22669201ad4f391c8f`.

## Границы приёмки

- Локальный test harness проверил production App/RenderRoot, редактор, File Gate и действие истории. Это не установленный Windows EXE и не OS-level UI Automation.
- `scripts/test-windows-beta.ps1` прямо запрещает запуск на рабочем компьютере и требует GitHub-hosted Windows runner. Windows Beta/UIA прогон и Main PR gate не запускались; их результат не заявляется.
- GitHub `main` на момент сверки — `0eb59af2b50aea1c262d6afba9832196b6ea9276`, на один коммит впереди локального `HEAD`. Сравнение показало только изменения `.github/scripts/pr-queue-budget.cjs`, `.github/scripts/pr-queue-budget.test.cjs` и `docs/engineering/AKULA-PR-QUEUE-BUDGET.md`; они не затрагивают Native-код задачи.
- GitHub поиск по `PC-DESK-01` и по `offline markdown history restore` не нашёл связанного PR. #686 закрыт без слияния и относится к переключению папок; #694 остаётся Draft по P2P и не пересекается с этой задачей.
- В test log есть нефатальные сообщения о занятости lock поискового индекса при пересоздании app harness и об отсутствующем `.trash` корне тестовой папки. Проверка заметки, истории и SHA при этом прошла; сообщения сохранены в журнале.
- Код и отчёт сохранены локально. PR, push, merge и запись в Notion не выполнялись. Карточка Notion остаётся без изменения.

**Следующая задача пакета A:** PC-DESK-02 — сохранить редактор и несохранённый текст при переключении рабочих папок, конфликте и ошибке записи.



---

## Publication update — 2026-10-24

This report has been published in the documentation-only one-PC evidence PR [#699](https://github.com/Lalalalendia/Akula/pull/699). The report records local evidence and its stated limits; this documentation PR does not include the underlying application source changes. PC-FG-01 implementation remains in Draft [#703](https://github.com/Lalalalendia/Akula/pull/703); PC-FG-02 test code/report are in Draft [#704](https://github.com/Lalalalendia/Akula/pull/704). No PR was merged.
