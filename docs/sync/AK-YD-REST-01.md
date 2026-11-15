# AK-YD-REST-01: personal Yandex Disk REST read-only groundwork

Status: implementation candidate; no production sync, no OAuth UI, no credentials saved.

This is **Yandex Disk** (`cloud-api.yandex.net`), not Yandex Cloud Object Storage (`storage.yandexcloud.net`).

## Boundary

- Compile with `--features sync-yandex-disk-rest` only. The installed binary does not enable this feature.
- Use a caller-supplied short-lived OAuth token for `cloud_api:disk.app_folder` access. Token acquisition/rotation through an OS vault is a separate PR.
- A single fixed HTTPS origin; GET-only metadata request; redirects disabled. No arbitrary URL, hostname or path.
- Root `app:/` and descendants only; reject unsafe path segments before performing a request.
- One bounded page per call, limited response bytes, 4-second connect and 12-second read/write timeouts.
- Reject missing/inconsistent metadata, duplicate paths, malformed or outside-root entries, bad offsets and empty nonterminal pages.
- Never treat an offset-based listing as a stable snapshot. This module explicitly does **not** implement `RemoteStore` or issue `VerifiedAbsent`, tombstones, `mark_applied`, `ConfirmedBaseline` or file operations.
- Yandex MD5 metadata is an untrusted provider hint; not BLAKE3 integrity evidence.
- No callback from this module can mutate local files, remote files, SQLite sync baseline, or settings.

## Probe invocation

```sh
cd aquilum-app
cargo test --locked -p aquilum-core --features sync-yandex-disk-rest yandex_disk_rest::tests
cargo check --locked -p aquilum-core --features sync-yandex-disk-rest
```

The Rust unit tests are offline and use fabricated metadata bodies. Actual API behavior and feature-specific CI must be validated before the PR is accepted.

## Next increments

1. Windows/Linux protected OAuth storage and browser-based Authorization Code + PKCE S256 with explicit request/redirect state; never reuse Yandex S3 credential targets.
2. Disposable read-only integration test with a test Yandex account; API pagination, rate limits, root identity and listing-change failure cases.
3. Import preview with explicit **unverified** status; do not map remote absence to deletion. Separate opt-in backup upload after independent safety review.
4. Evaluate whether any Yandex Disk API operation actually provides server-enforced create-if-absent / conditional update. Do not approximate compare-and-swap with HEAD followed by PUT.
5. Two-client immutable-log design and negative tests before any bidirectional mutation gate.

Primary references:
- https://yandex.com/dev/disk-api/doc/en/
- https://yandex.com/dev/id/doc/en/concepts/authorization-code
- https://github.com/remotely-save/remotely-save/blob/master/src/fsAll.ts (Apache-2.0)
- https://github.com/remotely-save/remotely-save/blob/master/LICENSE (PRO is PolyForm Strict)
- https://github.com/Lalalalendia/Akula/blob/main/aquilum-app/core/src/sync/provider.rs

Existing programs: Notion SYNC 00-11, STOR-T-01/02/21. This is an opt-in transport boundary, **not** completion of SYNC-08.
