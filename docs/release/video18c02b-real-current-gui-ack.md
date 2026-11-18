# VIDEO-18c-02b-01 — ACK от настоящего защищённого GUI (NON-RELEASE)

**Статус:** лабораторная PR-проверка. **Не выпуск, не подтверждение обновления и не owner-signed candidate.**

## Зачем

PR #1002 добавил отправку `AKULA-WIN-FIRST-PRESENTED-V1` только после непустой `Host::redraw → output.present → reveal` в экспериментальном Windows GUI. PR #1003 проверил другой конец IPC с настоящим отдельным дочерним процессом, но ребёнок был лабораторным EXE. Отдельно каждую часть проверили, **совместно с настоящей Akula — ещё нет**.

## Реальный тест

1. Существующий VIDEO-18b-16 Windows runner собирает **пять настоящих PE** из закреплённых источников и через **два NSIS** создаёт защищённую лабораторную установку `%ProgramFiles%\AkulaVideo18bRealNsisLab-<32hex>`: пять старых PE в корне и пять PE в `current/`. Все SHA сверяются, повреждение одного файла отвергается и восстанавливается.
2. Новый `labs/video-lite/video18c02b_real_gui_ack.rs` запускается только на GitHub-hosted Windows `pull_request`. Он проверяет точный GUID-root, отсутствие reparse/symlink, закрытый allowlist пяти PE и отдельную папку `current`. Настоящий `current/akula-app.exe` собран с `installed,video-rust-bundle,video-ui-e2e`.
3. Внешний тестовый родитель создаёт новый приватный Win32 pipe. Read-HANDLE не наследуется; **только** write-HANDLE разрешён конкретному `CreateProcessW` через `STARTUPINFOEX / PROC_THREAD_ATTRIBUTE_HANDLE_LIST`. Новый nonce получен через `BCryptGenRandom`, PID контроллера из ОС. Child environment содержит challenge fields и **тестовый** digest, который равен SHA установленного EXE, ранее проверенному PowerShell, но **не является подписью пятикомпонентного поколения**.
4. Дочерний процесс — настоящая Akula из защищённого `current/`, со свежим note-vault и профилем только под `RUNNER_TEMP`. Родитель закрепляет OS Child HANDLE/PID/creation time. Отдельная Windows `EnumWindows`-проверка требует видимое принадлежащее тому же PID окно Akula достаточных размеров и с корректным заголовком. Это независимое свидетельство **наличия видимого окна**, но не пиксельная аттестация compositor.
5. За 75 секунд родитель получает не более 256 байт через `PeekNamedPipe` и читает **только доступные байты**. Квитанция должна в точности совпасть с `AKULA-WIN-FIRST-PRESENTED-V1\tnonce\tversion\tdigest\tchild-PID\n`, с проверкой процесса OS и создания. Второй тест сознательно подменяет ожидаемый nonce, и тот же реальный GUI **не должен** получить разрешение. После каждого случая окно закрывается `WM_CLOSE`, процесс должен выйти; на ошибке только лабораторный child завершается принудительно.
6. Далее исходный VIDEO-18b-16 стенд продолжает обычную проверку standard-user ACL (10 отказов), двух NSIS-раскладок, полного удаления пяти PE и сохранения посторонних локальных файлов. Дополнительное source/negative CI запрещает ослабить `HANDLE_LIST`, проверку PID/creation identity, bounded reader и release HOLD.

## Правила приёмки

- [ ] Exact HEAD Application CI / no-growth / Main PR gate SUCCESS.
- [ ] Source/negative tests VIDEO-18c-02b-01 SUCCESS.
- [ ] Настоящий Windows GUI выдал квитанцию в закрытый родительский канал, OS window/PID/creation identity совпали; неверный nonce отклонён.
- [ ] Первоначальная VIDEO-18b-16 versioned NSIS приёмка (оба поколения + ACL + uninstall) осталась SUCCESS.
- [ ] Отдельное наблюдение post-merge, если соответствующий workflow запускается; различать skipped и full SUCCESS.

## Граница безопасности

Этот шаг **не** проверяет подпись издателя, подписанный манифест **пяти** EXE, этапы `rar2`, атомарную активацию, `confirm_candidate`, восстановление после сбоя питания, реальный публичный installer и визуальные пиксели композиции. Challenge в тестовом окружении передаётся дочернему процессу через приватный environment block; он связывает ответ с конкретным процессом, но **не гарантирует скрытность от привилегированного наблюдателя**. Полноценный trusted supervisor с проверкой токена, подписанной генерации, AV/locked PE и защитой от гонки наследования HANDLE — отдельные P0.

До 02b-02/03 и дальнейших релизных проверок **VIDEO_FIVE_CHANNEL_READY=false / VIDEO-18 RELEASE HOLD**.
