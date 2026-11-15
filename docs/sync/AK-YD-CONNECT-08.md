# AK-YD-CONNECT-08 — two-phase native Yandex Disk read-only connector

**Scope:** separate non-shipping coordinator using merged #947 (native OS keyring), #948 (Yandex REST/OAuth), and #949 (strict loopback callback). No second Sync Core, no user notes.

## User action sequence

1. User chooses one local Vault UUID and profile. Registered Yandex OAuth client ID and exact registered loopback port must come from product configuration.
2. Construct ConnectSession; reject invalid scope before opening a local listener; prepare one use PKCE authorize URL.
3. Host opens **system browser** explicitly; a dedicated worker awaits bounded loopback callback and exchanges code at fixed Yandex HTTPS token endpoint.
4. Using that same ephemeral access token, read Yandex ID id+client_id and compare the registered client ID; then request first app:/ root metadata page.
5. On a 200 root observation and presence of refresh token, return ReadOnlyReview. The UI must show the observed Yandex account ID for independent confirmation. Nothing saved so far.
6. Only a separate explicit confirmation of exactly that account ID, within 120s, calls YandexCredentialVault::system() and saves refresh token using exact account+app+Vault+profile fingerprint and readback verification.
7. The resulting receipt always reports Sync disabled. Absence or errors in account, app root, token, keyring, timeout, mismatched confirmation => no newly persisted refresh token.

## Safety and incomplete work

- Existing Yandex app-folder OAuth scope can authorize writes by the service; this **implementation** only makes read-only API calls.
- Offset-based listing is not a verified consistent remote snapshot. A successful root observation is not evidence that absent files were deleted.
- No default feature activation, OS browser opening, native settings button, background refresh, Sync provider mutation, local file writes, remote upload/delete or release.
- Production connects only to fixed official origins, never test URLs. Private test ports mock the coordinator after PKCE; no live account or credentials required.
- Tests verify ordering: no secret save before root+identity and explicit matching user approval; invalid local profile, wrong account, missing refresh, missing root and expired review fail closed.
- The UI requires separately registered Yandex OAuth App (client ID and redirect), a non-shipping opt-in host bridge, and a disposable-account real login test before enablement.

## CI

```sh
cargo test --manifest-path modules/yandex-disk-connect-lab/Cargo.toml
cargo check --manifest-path modules/yandex-disk-connect-lab/Cargo.toml --features system-store
```

Direct Windows/Linux CI owner and required Application checks. New module registered in mandatory ci-rust-scope.test.cjs list; no check bypass.

Sources: https://yandex.ru/dev/id/doc/ru/codes/code-url ; https://yandex.ru/dev/id/doc/ru/register-auth