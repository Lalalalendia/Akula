# M8 — один сервис для подготовленных источников (Эрида и Литэротика)

## Что реально работает

M8 соединяет реализованный M5–M7 конвейер с **постоянной SQLite-очередью,
локальным HTTP API и отдельным воркером**. Никаких новых копий парсера,
каталога, механизма Markdown/S3 и Erida importer здесь нет.

```text
оператор подтверждает источник и список страниц
    ↓
POST /v1/jobs (selected-work-v1) → SQLite очередь
    ↓                                  ↳ job_id / статус / повторная попытка
создание immutable snapshot в state/approved-snapshots/<plan_sha>/
    ↓
один воркер, глобальный lock + SQLite page ledger
    ↓
существующий M5/M7: RAW → PARSED → quality gate →
M4 publisher → immutable S3/local objects → current CAS → verified handoff
    ↓
существующий M6/M7: явный Erida export / read-only preview / opt-in staging
    ↓
подтверждение пользователя в Эриде (НИКОГДА автоматически)
```

Поддерживаемые **локальные** адаптеры входа: `fixture-chapter-html-v1` и
`literotica-saved-story-v1`. Второй понимает только сохранённые HTML-файлы,
проверенные схемой M7; сетевых запросов к Literotica нет. Расширение новыми
профилями реализуется через `SourceAdapter` и `AdapterRegistry` при
развёртывании, а не удалённой загрузкой кода из HTTP.

## Запуск на Linux / POSIX

```bash
cd experiments/unified-ingest-m5
python -m pip install -e '.[test]'

# Только для демонстрационного, синтетического approved-plan.json:
mkdir -p /tmp/ingest-m8/approved/demo-story
cp examples/approved-work/*.html /tmp/ingest-m8/approved/demo-story/

# Опция 1: CLI без сетевого сервера
python -m discovery_lab.service_cli submit \
  --state /tmp/ingest-m8/state --approved-root /tmp/ingest-m8/approved \
  --plan examples/approved-work/approved-plan.json
python -m discovery_lab.service_cli once \
  --state /tmp/ingest-m8/state --approved-root /tmp/ingest-m8/approved
python -m discovery_lab.service_cli list \
  --state /tmp/ingest-m8/state --approved-root /tmp/ingest-m8/approved

# Опция 2: локальный API, с автоматическим воркером
export INGEST_API_TOKEN="$(python -c 'import secrets; print(secrets.token_urlsafe(36))')"
python -m discovery_lab.service_cli serve --port 8765 \
  --state /tmp/ingest-m8/state --approved-root /tmp/ingest-m8/approved

# Из второго терминала (токен берётся из того же защищённого окружения):
curl -fsS -H "Authorization: Bearer $INGEST_API_TOKEN" \
  http://127.0.0.1:8765/v1/jobs
```

**Важно:** HTTP-сервер намеренно слушает только 127.0.0.1 и для всех
операций, кроме `/healthz`, требует bearer-токен от 24 символов.
Для сетевого доступа нужен отдельный аутентифицированный TLS-шлюз,
его настройка в M8 не выполняется. Credentials S3 и токен нельзя
помещать в URL, тексты заданий или коммиты.

## Контракт API

- `GET /healthz`: простая проверка живости, без секретов.
- `GET /v1/adapters`: разрешённые профили (никакой динамической загрузки кода).
- `POST /v1/jobs`: `{"plan": <selected-work-v1 JSON>, "idempotency_key":"optional-id"}`.
  Возвращает `201` и объект задания, либо `200` при повторе.
- `GET /v1/jobs`: до 50 последних заданий без исходных текстов.
- `GET /v1/jobs/<job_id>`: статус, результаты, ссылки на immutable объекты.
- `POST /v1/jobs/<job_id>/cancel`: только QUEUED, не прерывает идущую работу.
- `POST /v1/jobs/<job_id>/retry`: отменённое или ошибочное задание; **запрещено**,
  если страница требует разбора причины ошибки в существующем SQLite page ledger.

Пример запроса: тело — JSON с ключом `plan`, полностью совпадающим с
`examples/approved-work/approved-plan.json`. **Ни один endpoint не принимает
произвольный путь к файлу, HTTP-адрес для скачивания, Python-модуль или команду
оболочки.** Input snapshots должны находиться в управляемом оператором
`<approved-root>/<source_id>/<filename>`; при постановке в очередь копируются
в content-versioned файловую область и не зависят от последующих правок файлов.

Состояния: QUEUED → RUNNING → COMPLETED / NEEDS_REVIEW. Есть CANCELLED.
Неизменённый `work_id` и план всегда попадают в то же задание. При попытке
незаметно изменить каталог для этого work_id выдается 409; необходим новый
контролируемый этап миграции, а не переписывание старого receipt.

## Восстановление

- SQLite WAL хранит задания, счетчик попыток, статус и содержимое выбранного
  manifest, но **не тела глав**.
- `worker.lock` (POSIX flock) разрешает только одному исполнителю выполнять
  задания в данном state-каталоге. После остановки процесса lock освобождается.
- Следующий процесс восстанавливает заброшенные RUNNING и page leases
  FETCHING/PARSING, затем использует идемпотентную публикацию M4/M5.
- Устаревший SQLite job token не может дописать результат повторно; page ledger
  отдельно защищает запись старым токеном.
- Ошибочная, неподтверждённая или неполная страница остаётся NEEDS_REVIEW,
  а не автоматически объявляется завершённой.

## S3

Файловое immutable object storage включено по умолчанию. Настоящий S3
включается **оператором** с `--s3-bucket`, `--s3-endpoint` и доступными
переменными AWS SDK. API-клиент не управляет endpoint или секретами.
Код повторно использует M4 `S3ObjectSink` с проверкой SHA-256, create-only
и CAS; его прежние ограничения на большие объекты и API остаются в силе.
В текущих локальных интеграционных тестах M8 проверен с файловым backend;
его полный цикл через настоящий MinIO **требует отдельной проверки**.

## Приёмка

```bash
python -m unittest discover -s tests -p test_m8_service.py -v
python -m unittest discover -s tests -p 'test_m[5678]*.py' -v
```

Новые тесты M8 проверяют: SQLite re-open, ID/idempotency, race из параллельных
submit, сохранение snapshot, аварийный requeue + fencing токенов, контроль
активного worker flock, M5 публикацию и readback, preloaded RAW, cancel/retry,
API/HTTP auth, отсутствие staging эндпоинтов, path traversal и незнакомые профили.

## Что не следует обещать

Это **изолированный Linux/POSIX сервис первой версии**, не production
внедрение в Эриду или сайт Литэротики. Не выполняются crawler discovery,
проверка robots/ToS, браузерная загрузка, перевод, согласие пользователя,
автоматическое Erida confirm или реальная массовая выгрузка.
Дальнейшее подключение site-specific разрешённого FetchSpec/FetchReceipt
делается за входным интерфейсом, с текущими договорами доступа/квотами
Эриды и M4 quality gate без их замены.
