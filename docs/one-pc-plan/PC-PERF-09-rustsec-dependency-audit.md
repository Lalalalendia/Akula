# PC-PERF-09 — RustSec-аудит зависимостей

Локальное доказательство по [AKULA-ONE-PC-PLAN-01](https://app.notion.com/p/40332a84beec8199b005f7a239210763), собрано 2026-10-24. Notion не изменялся.

## Входное состояние и область

- Репозиторий: `D:\mobakula`, ветка `main`, входной и текущий `HEAD`: `530fe3d6794ee28d2c7098af1266ff5340903543`.
- Рабочее дерево содержит ранее начатые изменения PC-FG/P2P/search; они сохранены. Коммит, fetch, rebase, push, PR и merge не выполнялись.
- GitHub `main` на контрольном чтении: `0eb59af2b50aea1c262d6afba9832196b6ea9276`, на один коммит впереди; он меняет только лимит очереди PR. Локальную ветку не перемещал.
- PR #442 остаётся открытым Draft: он добавляет диагностический `index-stage-probe` и workflow; файл `core/Cargo.toml` пересекается, но изменения не реализуют RustSec исправление. PR #395 — отдельная проба семантического поиска. Идентичного исправления зависимости в этих PR не найдено.
- Исполняемая очередь Notion относит PC-PERF-09 к первому незавершённому A0/P0 пункту; зависимость PC-00.02 подтверждена локальным отчётом.

В репозитории есть два канонических lockfile, которые сканирует `dependency-security.yml`: `aquilum-app/Cargo.lock` и `aquilum-app/host-broker-lab/Cargo.lock`. Также обнаружены отдельные рабочие lockfile изолированных UI-проб `native/probes/freya/` и `native/probes/masonry/`. Они не входят в workspace приложения и не указаны в workflow, но дополнительно просканированы и обновлены по найденному advisory.

## Найденные проблемы и исправления

На зафиксированном уязвимом baseline и при свежем сканировании базой RustSec `8a5b8090ef20c9c1b425f3373f28b0b23cc5c7a0` выявлено:

- `event-listener 5.4.1`, [RUSTSEC-2026-0221](https://rustsec.org/advisories/RUSTSEC-2026-0221), и `lru 0.16.4`, [RUSTSEC-2026-0253](https://rustsec.org/advisories/RUSTSEC-2026-0253), были в baseline; первый обновлён до `5.4.2`, обновление Tantivy до `0.27.0` заменило второй на `lru 0.18.5`.
- `crypto-bigint 0.5.5`, RUSTSEC-2026-0387, в необязательной ветке AWS SigV4a. `aws-sigv4 1.6.3` всё ещё выбирал уязвимую версию при включённом SigV4a ([исправление RustCrypto](https://github.com/RustCrypto/crypto-bigint/pull/1365)). В текущем S3-контракте нет MRAP/SigV4a использования; AWS требует SigV4A для Multi-Region Access Points ([ограничения MRAP](https://docs.aws.amazon.com/AmazonS3/latest/userguide/MultiRegionAccessPointRestrictions.html)). Поэтому SigV4a удалён из feature-листов S3 и STS. Региональный `sync-s3` остаётся собираемым. Добавлен комментарий в manifest с причиной запрета на повторное включение до чистого аудита.
- Новый `tokio 1.53.2`, [RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html) (unsound `AsyncFd` constructors); обновлён до исправленной `1.53.3` в основном lockfile и двух lockfile проб.

Уязвимый baseline из исходного `HEAD` сохранён и использован как отрицательный контроль. Его аудит завершается ошибкой: RustSec находит `crypto-bigint 0.5.5`, а режим `--deny unsound` также отвергает старые `event-listener` и `lru`.

## Проверки

База advisories: `D:\mobakula\.artifacts\one-pc-cargo-home\advisory-db`, commit `8a5b8090ef20c9c1b425f3373f28b0b23cc5c7a0`.

Команда для каждого lockfile:

```powershell
cargo audit --deny unsound --no-yanked --no-fetch --db D:\mobakula\.artifacts\one-pc-cargo-home\advisory-db --file <Cargo.lock>
```

| Lockfile | Итог | SHA-256 |
| --- | --- | --- |
| `aquilum-app/Cargo.lock` | exit 0; уязвимостей и unsound advisories нет; осталось 8 разрешённых предупреждений | `33103743A49376BFAA14C9BEE40389A03AD2CF7EDE31158537440A64FF012319` |
| `aquilum-app/host-broker-lab/Cargo.lock` | exit 0; уязвимостей и unsound advisories нет | `CAD27732FEE7D3FEBB7764CB120FBB9145D2BD49E4E096663DC7026BCB6BF51B` |
| `aquilum-app/native/probes/freya/Cargo.lock` | exit 0; уязвимостей и unsound advisories нет; 5 разрешённых предупреждений | `3D3B8C359E3BBCC162DACD4D03FDEEA98B1CC76B28FAD5DD1DD4812CAD68ADD1` |
| `aquilum-app/native/probes/masonry/Cargo.lock` | exit 0; уязвимостей и unsound advisories нет; 3 разрешённых предупреждения | `BC626B1084F4CA5808E61B91264887EEA4921DA50DEB065C5D9B9A4A2D98CB28` |

Дополнительная регрессия AWS SDK проверена на текущем рабочем дереве и зафиксированном lockfile:

```powershell
cargo +1.97.1 check --manifest-path aquilum-app/Cargo.toml --locked -p aquilum-core --features sync-s3
```

Результат: **PASS**, exit 0. Проверка подтверждает компиляцию текущего регионального `sync-s3` feature; она не запускает внешний S3-сервис и не подтверждает MRAP.

Для Tantivy-обновления выполнен изолированный запуск поисковых тестов на snapshot без незавершённых изменений File Gate: **361 passed, 0 failed, 14 ignored**. Лог: `.artifacts/one-pc-checks/PC-PERF-09-20261024/search-tests-isolated-current-fix.log`.

Негативный контроль: исходный lockfile из `HEAD` с `--deny unsound` завершился кодом 1 и обнаружил `crypto-bigint 0.5.5` / RUSTSEC-2026-0387, `event-listener 5.4.1` / RUSTSEC-2026-0221 и `lru 0.16.4` / RUSTSEC-2026-0253.

Полные журналы находятся в `.artifacts/one-pc-checks/PC-PERF-09-20261024/`:

- `final-audit-main-deny-unsound.stdout.log`
- `final-audit-host-broker-deny-unsound.stdout.log`
- `final-audit-freya-probe-deny-unsound.stdout.log`
- `final-audit-masonry-probe-deny-unsound.stdout.log`
- `final-audit-vulnerable-baseline-deny-unsound.stdout.log`
- `final-sync-s3-check-online.log`

Секреты и пользовательские данные не использовались; audit-логи содержат только сведения о зависимостях и advisory.

## Границы результата

- В `dependency-security.yml` по-прежнему проверяются два канонических lockfile приложения и host-broker. UI-пробы не подключены к этому watchdog; проверка их CI-маршрута относится к следующей задаче PC-PERF-10.
- GitHub Actions, Main PR gate и merge не запускались: изменения остаются локальными и не закоммичены. Точное CI подтверждение последнего HEAD не заявляется.
- В Notion карточка PC-PERF-09 остаётся `В очереди` согласно инструкции не менять Notion без отдельного указания.
- SigV4a/Multi-Region Access Points остаются вне текущей S3-поверхности; включать их следует только вместе с обновлением SDK-графа и повторным RustSec-аудитом.

Следующая зависимость-доступная задача после локально подтверждённых PC-FG-01/02/03: **PC-FG-04 — symlink/junction/reparse и гонка подмены пути**.



---

## Publication update — 2026-10-24

This report has been published in the documentation-only one-PC evidence PR [#699](https://github.com/Lalalalendia/Akula/pull/699). The report records local evidence and its stated limits; this documentation PR does not include the underlying application source changes. PC-FG-01 implementation remains in Draft [#703](https://github.com/Lalalalendia/Akula/pull/703); PC-FG-02 test code/report are in Draft [#704](https://github.com/Lalalalendia/Akula/pull/704). No PR was merged.
