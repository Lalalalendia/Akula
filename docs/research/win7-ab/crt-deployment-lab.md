# PC-DESK-08 — Windows 7: убрать обязательный UCRT/VC++ Redistributable

## Обнаруженный недостаток прежней успешной Win7-сборки

В артефакте [Win7 LAB #40327304247](https://github.com/Lalalendia/Akula/actions/runs/40327304247/artifacts/12655146196) уже был готов обычный NSIS и успешно прошли запуски на Windows Server 2022. Однако независимые отчёты `win7-lab-output/pe-audit/full-audit.json` и `win7-full-results/shiva-audit/full-audit.json` показали, что оба EXE импортировали `api-ms-win-crt-*.dll` и `vcruntime140.dll`. На чистой Windows 7 SP1 они не гарантированно установлены; прежняя проверка «нет API Windows 8+» не считала такой runtime dependency ошибкой.

- Microsoft: [Universal CRT deployment](https://learn.microsoft.com/en-us/cpp/windows/universal-crt-deployment?view=msvc-170) — UCRT входит в Windows 10+, для Windows 7 SP1 отдельно распространяется через обновление KB2999226.
- Microsoft: [KB2999226 for Windows 7 SP1](https://support.microsoft.com/en-gb/servicing/os/windows/2020/04/update-for-universal-c-runtime-in-windows).
- Microsoft: [Visual C++ Redistributable support](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist?view=msvc-170) — новейший v14 Redist для VS 2026 поддерживает только Windows 10/11 и серверные аналоги.
- Rust: [Static and dynamic C runtimes](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes) — `-C target-feature=+crt-static` выбирает статическую линковку CRT на MSVC, но результат необходимо проверять по самому бинарнику.

## Эксперимент

[Ветка `lab/win7-static-crt-no-ucrt-20261026`](https://github.com/Lalalendia/Akula/tree/lab/win7-static-crt-no-ucrt-20261026) пробует `-C target-feature=+crt-static` вместе с уже действующим Win7-target и `-Z build-std=std,panic_abort` **на полном приложении** с фичами `installed,s3-pair-beta` и на закреплённой версии Shiva PDF. Одинаковый флаг применяется в лабораторной и защищённой подписываемой сборке.

Добавлен отчёт `external_crt_imported_dlls` по полному import table. Лаборатория, проверка NSIS и защищённый signed stage отказывают, если в Akula или Shiva остались динамические `api-ms-win-crt-*`, `ucrtbase.dll`, `vcruntime*.dll` или `msvcp*.dll`. Никакой подмены DLL, скачивания сторонних бинарников и автоматической публикации нет.

## Приёмка

1. Оба полнофункциональных EXE успешно компилируются с одним Win7-target, без отказов Rust/C/C++ native dependencies.
2. В обоих PE-аудитах `external_crt_imported_dlls=[]`, `known_post_win7_imports=[]`; исходный и установленный файлы совпадают по SHA256.
3. Обычный NSIS собирается, устанавливается, GUI/редактирование/перезапуск и Shiva PDF проходят на Windows Server 2022; затем **реальная Windows 7 SP1 x64** по PC-DESK-08 с 20 перекрытиями.
4. Отсутствие динамического CRT **не доказывает** полноту совместимости с Windows 7: остаются возможные задержанные загрузки API, GPU/IME, проблемы современных TLS-соединений, драйверов и подписи.
5. После реального Win7 QA и Windows10/11 regression необходимо выпустить подписанный пакет версии **выше v0.3.14** только через защищённый релизный workflow. Win7 — ОС без поддержки безопасности Microsoft.

Этот документ — описание эксперимента, не объявление user readiness. При неуспешной статической линковке сохранить текущий production main и отдельно оценить официальное развёртывание UCRT/VC Redist через установщик.
