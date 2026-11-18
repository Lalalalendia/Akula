# VIDEO-18c-02b-02b-02a — два различных исходно собранных Windows GUI (NON-RELEASE)

**Состояние:** отдельная лабораторная проверка в PR, не выпуск, не переключение установки и не `rar2` commit.

## Предпосылка

Принятый #1008 проверил единую цепочку: пять настоящих PE в защищённом `current/` → одноразовая подпись Minisign → `SignedFive::verify_stage` → запуск настоящего GUI Akula → `AKULA-WIN-FIRST-PRESENTED-V1` с SHA подписанного манифеста. Но **legacy flat и current имели одинаковые SHA PE**, поэтому тест не доказывал обновления между поколениями.

Дополнительное ограничение: `aquilum-app/native/Cargo.toml` в старом `ec6d6d48a78e60fca7a6f810b825418500bad64b` и в новом `main` декларирует **одну и ту же пакетную версию 0.3.20**. Метка 0.3.21 в тестовом манифесте не является подтверждением версии установленного исполняемого файла.

## Что этот PR проверяет

1. В том же GitHub-hosted Windows `pull_request` job под отдельным `actions/checkout@v4` загружается **ровно** исторический source commit `ec6d6d48a78e60fca7a6f810b825418500bad64b`. Новый код остаётся закреплён за точным `github.event.pull_request.head.sha`. Исторический источник должен **не** содержать `windows_first_gui_ack.rs`, новый — содержать. Деревья `aquilum-app/native/Cargo.toml` и отслеживаемого `aquilum-app/Cargo.lock` сверяются по Git blob и фактическим байтам.
2. Из исторического checkout собирается настоящий `aquilum-native.exe` через `cargo build --release --locked -p aquilum-native --features installed,video-rust-bundle,video-ui-e2e`, в **отдельном** `CARGO_TARGET_DIR` только под `RUNNER_TEMP`. Современная Akula уже собирается тем же workflow с теми же feature flags из точного PR HEAD.
3. Скрипт `scripts/windows-video18c02b02b02a-distinct-native.ps1` требует: разные SHA-256 бинарников, два genuine PE/MZ без reparse, две закреплённые версии исходников, неизменённые Cargo.toml/Cargo.lock, фактическую `0.3.20` обеих и только hosted Windows PR. Никакие хвостовые байты EXE искусственно не меняются.
4. Отдельный старый `aquilum-native.exe` **запускается** с новым изолированным профилем и заметкой под `RUNNER_TEMP`. Доверенный `.NET Process` с собственным process handle проверяет уникальный OS PID, creation time, непустое видимое окно с заголовком Akula, ограниченное ожидание и корректное закрытие; после выхода SHA обоих PE и приватной заметки не должны измениться.
5. **Далее без изменений** продолжаются исходные NSIS, ephemeral Minisign, проверка подписанного new/current five-PE manifest, реальные first-frame GUI ACK и отрицательные сценарии, десять ACL-denials, полный uninstall с сохранением пользовательских данных. Контроллер не открывает `rar2` journal и не подтверждает установку.
6. Node source/negative contract запрещает заменить старый commit на `main`, ослабить сравнение SHA, убрать закрепление PID/StartTime/видимого окна или контроль безопасности исходников.

## Что результат не доказывает

Старая Akula запускается **в отдельном временном окружении**, а не из защищённого `legacy flat` installer; новая по-прежнему запускается из защищённого `current/`. Тест демонстрирует **различие настоящих исходно собранных PE** и работоспособность старого GUI, но не переключение между ними внутри одной установки. Текущий подписанный manifest #1008 описывает только пять файлов `current/`, а старый бинарник ещё **не** включён в отличный по SHA подписанный legacy manifest. Бинарники source-pinned; их полная побитовая воспроизводимость в независимых CI-запусках пока не доказана. Семантическое повышение версии выше 0.3.20 также не доказано.

Следующий отдельный 02b-02b-02b — два distinct five-PE manifests, реально отличающиеся `legacy` / `current` под Program Files, cryptographic binding новой генерации к actual GUI ACK и verified old fallback. Затем 02b-03 — атомарная активация `rar2` с позитивным ACK→confirm и полным rollback всех пяти PE на negatives/process crash.

Никакого пользовательского updater, открытого сервера, CI publishing, реальной подписи издателя или релизного NSIS. `VIDEO_FIVE_CHANNEL_READY=false` и **VIDEO-18 RELEASE HOLD**.

## Критерии приёмки

- [ ] Exact HEAD source/negative contracts и Application CI SUCCESS.
- [ ] Настоящая Windows-компиляция старого и нового исходных состояний, разные SHA-256 PE без искусственной мутации.
- [ ] `VIDEO18C02B02B02A_OLD_GUI_VISIBLE_PASS` — реальный старый процесс, pinned PID, видимое окно, clean exit, сохранность заметки.
- [ ] Существующая подпись текущего пяти-PE manifest и подтверждение реального new GUI ACK не регрессировали.
- [ ] NSIS/ACL/uninstall, signed-five, `rar2`, VIDEO-05 SUCCESS; merge только после exact-HEAD приёмки, выпуск по-прежнему запрещён.
