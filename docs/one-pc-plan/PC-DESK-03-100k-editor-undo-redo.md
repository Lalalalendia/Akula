# PC-DESK-03 — 30k/100k строк, массовая вставка и восстановление заметки

**План:** [AKULA-ONE-PC-PLAN-01](https://app.notion.com/p/40332a84beec8199b005f7a239210763) · **карточка:** [PC-DESK-03](https://app.notion.com/p/40332a84beec81a99df9d408cfad9a29)<br>
**Проверка:** 24.10.2026, один физический Windows ПК, синтетическая заметка в изолированном App Harness.<br>
**Git:** ветка `main`; исходный и итоговый `HEAD` — `530fe3d6794ee28d2c7098af1266ff5340903543`. Изменения локальные, не закоммичены.

## Результат

Исправлен пропуск клавиатурных source-команд, пока layout редактора ожидает следующий кадр после массовой вставки. В этом состоянии Ctrl+A, Copy, Undo и Redo теперь обрабатываются по полному Rope; команды, которым нужна геометрия окна или таблицы, по-прежнему требуют layout. При большой вставке в windowed документе полный Rope остаётся каноническим, а видимое окно повторно монтируется вокруг конца вставки. Это предотвращает синхронную раскладку десятков тысяч строк при Undo.

Добавлены проверки:

1. Ctrl+A, массовая вставка, Ctrl+Z и Ctrl+Y на заметке из 100k строк; точные байты и SHA-256 проверены в редакторе, после каждого сохранения и после пересоздания приложения.
2. Ctrl+A и Ctrl+Z сразу после длинной вставки без промежуточного redraw.
3. Windowed document после восстановления всех 100k строк сохраняет ограниченное окно и полный исходный Rope.
4. Несохранённая правка первой вкладки записывается при открытии второй и остаётся точной после возврата к первой.

## Изменённые файлы

- `aquilum-app/native/src/ui/editor/widget/widget_events.rs`
- `aquilum-app/native/src/ui/editor/document/edit.rs`
- `aquilum-app/native/src/ui/editor/document/window.rs`
- `aquilum-app/native/src/ui/editor/document/window_tests.rs`
- `aquilum-app/native/src/ui/editor/tests/clipboard.rs`
- `aquilum-app/native/src/app/tests/notes/saving.rs`

## Точные байты и проверки

| Состояние | Длина | SHA-256 |
| --- | ---: | --- |
| Исходная заметка до вставки | 7,588,890 байт | `cb376884b6ec5c3d2eb34b7fbbd3617d674d0565f2cac54109cc7d6e8a918226` |
| Массовая вставка | 82 байта | `35f80b332c3892125284ccc17f215c65939c3b0de9cdd900e4a609f39a6b7fb5` |
| После Ctrl+Z и сохранения | 7,588,890 байт | совпал с исходником |
| После Ctrl+Y, сохранения и перезапуска | 82 байта | совпал с массовой вставкой |
| Черновик первой вкладки после переключения туда и обратно | 73 байта | `cfe3d837be49375f7dc891cef113df552b57637c09caee51b8e899e494261601` |

Исходная версия до теста переключения вкладок имела SHA-256 `57248747d7880ed18318f190337ecccadcafcd2c3fe09a2d6f21f730503a10c2`.

Команды (Rust `1.96.0`, `--locked --offline`):

```powershell
cargo +1.96.0 test --manifest-path aquilum-app/Cargo.toml --locked --offline -p aquilum-native --bin aquilum-native app::tests::notes::saving:: -- --nocapture
cargo +1.96.0 test --manifest-path aquilum-app/Cargo.toml --locked --offline -p aquilum-native --bin aquilum-native ui::editor::tests::clipboard:: -- --nocapture
cargo +1.96.0 test --manifest-path aquilum-app/Cargo.toml --locked --offline -p aquilum-native --bin aquilum-native ui::editor::document::window_tests:: -- --nocapture
git diff --check
```

Результаты: **8/8**, **5/5**, **9/9** тестов; `git diff --check` прошёл. Журналы:

- [native-saving-final.log](../../.artifacts/one-pc-checks/PC-DESK-03-20261024-local/native-saving-final.log), SHA-256 `47007d366febc5a01b36dae0741bd3424cc28e1761d9be8eba773533ae096bc2`
- [native-clipboard-final.log](../../.artifacts/one-pc-checks/PC-DESK-03-20261024-local/native-clipboard-final.log), SHA-256 `257a315b3858b84611bb728cb38946dcd74d0dc65ea50034f1c98f3d44d9d777`
- [native-window-final.log](../../.artifacts/one-pc-checks/PC-DESK-03-20261024-local/native-window-final.log), SHA-256 `c105b9cdf0fe61e74c0e68213c1f8fb5b1ec7b6f9da3c8be02c0c47ac08c271c`

В журнале suite сохранения есть нефатальные диагностические сообщения тестового harness про отсутствующий корень `.trash` и занятый lock поискового индекса; все проверки сохранения и хешей прошли.

## Граница приёмки

- **L1, локальный код и App Harness:** пройдены на одном Windows ПК с синтетическими данными.
- **Установленная Windows Beta и OS-level UI Automation:** не запускались.
- **GitHub Actions и Main PR gate:** не запускались; эти изменения находятся только в локальном checkout, поэтому удалённый workflow не мог проверить этот исходник.
- Связанные открытые PR #504 и #622 проверялись как контекст, но не обновлялись и не считаются слиянием этой реализации.
- **GitHub:** push, новый/обновлённый PR и merge не выполнялись.
- **Notion:** карточка не обновлялась; статус остаётся `В очереди`.

**Следующий пункт плана:** PC-DESK-04 — TXT в общем поиске, китайские/японские знаки без пустых квадратиков и полный охват русского/английского UI, включая установщик.



---

## Publication update — 2026-10-24

This report has been published in the documentation-only one-PC evidence PR [#699](https://github.com/Lalalalendia/Akula/pull/699). The report records local evidence and its stated limits; this documentation PR does not include the underlying application source changes. PC-FG-01 implementation remains in Draft [#703](https://github.com/Lalalalendia/Akula/pull/703); PC-FG-02 test code/report are in Draft [#704](https://github.com/Lalalalendia/Akula/pull/704). No PR was merged.
