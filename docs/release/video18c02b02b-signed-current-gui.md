# VIDEO-18c-02b-02b-01 — подписанный manifest пяти реальных PE и GUI ACK (NON-RELEASE)

**Статус:** нерелизная совместная лабораторная проверка. Не поставка, не операция обновления, не `rar2` commit.

## Почему новый тест нужен

Принятый #1006 проверяет получение first-frame ACK от настоящей Akula через наследуемый приватный Win32 HANDLE, но использует SHA одного GUI EXE. Принятый #1007 умеет проверить ACK по SHA **криптографически аутентифицированного manifest всех пяти PE**, но в реальном Windows GUI ещё не вызывался. Этот этап объединяет обе части **в одном нерелизном процессе**.

## Что проверяем

- Тот же disposable GitHub-hosted Windows PR runner собирает из закреплённых источников настоящие `akula-app.exe`, Shiva, Rust mux, yt-dlp и QuickJS. Два настоящих NSIS создают защищённый Program Files GUID-каталог с пятью PE в старой плоской раскладке и пятью PE в `current/`. Все файлы проходят точные SHA/size и проверку прав.
- Создаётся **новый однократный тестовый** Minisign 0.12 ключ из проверенного SHA-pinned CLI. Подписывается manifest schema=1, product=Akula, purpose=protected-video-five-exe, version=0.3.21, точный PR HEAD и tracked `aquilum-app/Cargo.lock` SHA, имена и SHA/размеры пяти исполняемых PE **без искусственного изменения байтов**. Независимый Minisign CLI проверяет эту подпись до вызова Rust.
- Изолированный `publish=false` Cargo crate `labs/video-lite/video18c02b02b-signed-gui` с временно генерируемым только в CI `Cargo.lock` выполняет `admit_hosted_lab_signature()`. Он проверяет PR HEAD/версию/Cargo.lock, размещение файлов подписи под `RUNNER_TEMP`, пути без reparse и `SignedFive::verify_stage(current)`.
- **Тот же процесс** с аутентифицированным `SignedFive` запускает реальный `current/akula-app.exe` через старый проверенный Win32 `STARTUPINFOEX/PROC_THREAD_ATTRIBUTE_HANDLE_LIST` родительский механизм. Получает ACK после первого непустого кадра, проверяет OS Child HANDLE/PID/creation identity, видимое окно и точное совпадение ACK с nonce, PID и `SignedFive::generation_sha256()`.
- Отдельные реальные GUI-запуски должны **отклонить** чужой nonce и ответ, в котором GUI запущен с неподписанным `generation SHA`; после проверки повторно сверяются все пять PE и исходный плоский набор.
- Источник с отрицательными Node-тестами не допускает обхода Minisign, проверки SHA пяти PE, OS PID/creation identity, read-only `SignedFive`, pinned CLI, точного HEAD или снижения RELEASE HOLD. Сохранены ten standard-user ACL denials и полный uninstall с сохранностью пользовательской заметки.

## Чего этот результат не доказывает

**В этой лаборатории legacy flat и новый current пока имеют одинаковые байты пяти EXE.** Это проверка правильной привязки настоящего GUI ACK к **подписанному manifest текущего запускаемого набора**, но НЕ доказательство переключения между двумя отличающимися исполняемыми поколениями. Указанная в подписанном manifest версия 0.3.21 является лабораторной меткой, а не доказанной поставляемой версией.

Нужно отдельно сделать 02b-02b-02: два разных реально исполняемых набора с воспроизводимой историей сборки, строгий version-floor, signed manifest только нового набора, повторная проверка SHA после staging/активации под OS-lock без TOCTOU. Потом 02b-03: atomically activate/confirm через pinned `rar2` в случае настоящего ACK, а при всех отрицательных случаях restore прежних пяти PE. Отдельные P0: reviewed `rar2 Cargo.lock`, power-loss, Windows 10/11, AV/locked files, реальные owner keys, привилегии сервиса.

`VIDEO_FIVE_CHANNEL_READY=false`. Не затрагивать штатный two-EXE updater, пользовательские установки, релизный канал, продуктивные данные и миграции. Ключи одноразовые, находятся только на ephemeral runner и удаляются вместе с тестовым staging.

## Приёмка

- [ ] На точном HEAD source/negative tests SUCCESS.
- [ ] На настоящем Windows runner собран новый crate, проверены настоящий Minisign CLI и установленная запускаемая пятёрка PE.
- [ ] `VIDEO18C02B02B_SIGNED_RUNNABLE_GUI_ACK_PASS`: подписанный manifest SHA совпал с first-frame ACK, bad nonce и bad digest отказали.
- [ ] Исходные NSIS/ACL/uninstall tests, Application CI, VIDEO-05 SUCCESS.
- [ ] Объединять только после доказательств, `RELEASE HOLD` не снимать.
