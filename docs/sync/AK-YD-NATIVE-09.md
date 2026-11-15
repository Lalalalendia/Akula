# AK-YD-NATIVE-09 — personal Yandex Disk opt-in native Settings onboarding

Status: candidate in a Draft PR, NOT shipped, NOT a completed two-way sync.

## Implemented for opt-in debug builds

- Adds a distinct **personal Yandex Disk REST** block in Settings → Sync, separate from existing **Yandex Cloud Object Storage S3** setup.
- Uses accepted, independently tested libraries: #947 native OS keystore, #948 personal Disk REST/OAuth, #949 loopback callback, #950 review-before-save coordinator.
- A product-registered public OAuth client ID and registered IPv4 loopback callback port must be supplied at **build time** via `AKULA_YANDEX_OAUTH_CLIENT_ID` and `AKULA_YANDEX_OAUTH_CALLBACK_PORT`. No arbitrary HTTP endpoints/credentials entered in Settings. Without these values the login button is disabled and the status explains why.
- The locally selected Workspace UUID comes from Akula's existing local `UiStateDatabase::list_workspaces()` record. This is **only a device-local OS secret namespace**, not a cloud VaultId or publisher proof.
- One explicit Login click spawns a background OS worker: bind local listener before opening the system browser; receive PKCE callback, exchange short-lived token, verify Yandex ID and OAuth app, perform a read-only `app:/` root metadata probe, prepare a `ReadOnlyReview`.
- The UI then shows Yandex account ID and demands a **second explicit click** naming the exact account. Only then is the refresh token sent to the OS keyring through a separate worker; no secrets in `AppConfig`, status strings, stdout/stderr or settings.
- On switching workspaces, pending account review is dropped. A delayed callback cannot become a review for the newly selected workspace, and the single-flight gate forbids overlapping browser requests. The OS keyring record remains partitioned by verified user ID, app, local Workspace UUID and profile.
- All statuses still say `SYNC_DISABLED`; no Sync Core authority, remote upload/delete, local file writes, scheduler or background refresh.
- Protects release boundary with compile-time `compile_error!` if the preview feature is compiled without debug assertions. Normal `installed` feature list is unchanged.
- Windows/Linux native offline CI tests and mandatory PR-level Application checks must both succeed.

## Prerequisites for actual user login

1. Register an Akula-owned Yandex ID OAuth application and exact `http://127.0.0.1:<port>/akula/yandex/callback` URI; authorize `cloud_api:disk.app_folder`. The public client ID and registered port are **not yet in Akula**.
2. Build non-shipping debug QA with that exact public ID/port. Verify on a disposable user-controlled Yandex Disk account. No OAuth secret supplied in repo or CI.
3. Add an explicit post-restart re-verification path for saved tokens and a local-forget/revoke UX before a normal release; do not assume a stored refresh token is permanent authorization.
4. Treat app-folder listing as `YandexDiskUnverifiedPage`. Do not infer deletion or allow automatic bidirectional file changes until independently proven concurrency protocol and File Gate acceptance.

## CI

```sh
cd aquilum-app
cargo check --locked -p aquilum-native --features installed,yandex-disk-readonly-preview
cargo test --locked -p aquilum-native --bin aquilum-native --features installed,yandex-disk-readonly-preview yandex_disk -- --test-threads=1
```

The normal Akula installed release remains unchanged, and Yandex Cloud S3 is a separate provider.
