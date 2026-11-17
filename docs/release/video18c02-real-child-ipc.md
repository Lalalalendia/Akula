# VIDEO-18c-02a — настоящий Windows child-process IPC (NON-RELEASE)

Статус: независимый лабораторный шаг после #1002. **Не** trusted update controller, **не** подтверждение первого GUI-кадра и **не** публичный выпуск.

## Задача

В #1002 доказан реальный Win32 pipe read/write внутри одного процесса и доставка только из точки первого непустого GUI redraw. Но GetHandleInformation(HANDLE_FLAG_INHERIT) не удостоверяет, что HANDLE действительно передан **другому** процессу контроллером. Этот пробел закрывает отдельная, физическая Win32 parent/child лаборатория.

## Реализация

- Один отдельный исходник labs/video-lite/video18c02_windows_child_ipc.rs компилируется через rustc на одноразовом GitHub-hosted Windows pull_request runner, без новой product-зависимости/Cargo.lock, публикации или инсталляции.
- Родитель создаёт две пары CreatePipe с наследуемыми write-handle и выключает наследование read-handle. Первый write разрешает через STARTUPINFOEX + PROC_THREAD_ATTRIBUTE_HANDLE_LIST. Второй write намеренно наследуем, но НЕ входит в allowlist; ребёнок должен отказать ему как доступной pipe.
- CreateProcessW запускает ровно тот же лабораторный EXE, закрепляет настоящий OS process HANDLE и PID. Nonce генерирует BCryptGenRandom; протокол содержит nonce, version, generation placeholder, реальный PID ребёнка. Полученная строка сравнивается с OS GetProcessId(HANDLE), а не с произвольным PID из текста.
- Родитель закрывает свою копию write, ограничивает ожидание дочернего процесса 15 секундами, отвергает oversized/duplicate/no data/неверный nonce и проверяет точный единичный receipt. Превышение deadline приводит к TerminateProcess лабораторного ребёнка. Пять рабочих компонентов Akula, реальный updater и пользовательские данные не изменяются.
- Source/negative контракт запрещает ослабление OS PID check, handle whitelist и 256-byte limit; не допускает product confirm/publish. Runner проверяет точный PR HEAD, а результаты сообщает отдельным маркером VIDEO18C02_REAL_PARENT_CHILD_PIPE_PASS.

## Что это НЕ доказывает

Это только транспортный межпроцессный тест. Дочерний процесс — отдельный лабораторный EXE, **не** новая подписанная Akula, событие ACK не привязано к реальному GUI frame, версия и SHA пока синтетические. Никакого вызова confirm_candidate, rar2-журнала, активации пяти PE, публикации или owner key. VIDEO-18 RELEASE HOLD остаётся.

## Следующая интеграция 02b

Защищённый controller вне current/ должен через реальный signed-five manifest, exact five SHA, OS install lock и durable rar2 journal закрепить запускаемый новый GUI child PROCESS HANDLE/creation identity и одноразовый nonce. Только настоящий GUI first-presented-frame receipt, сверенный с независимым GUI witness до deadline, может разрешить lab commit. Остальные сценарии — fail-closed rollback всех пяти PE. Отдельно до релиза: process kill/restart, power-cut, AV/locked PE, Windows 10/11 и owner-signed delivery.
