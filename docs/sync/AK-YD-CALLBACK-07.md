# AK-YD-CALLBACK-07 — bind an explicit Yandex browser redirect to loopback

Status: opt-in native protocol layer, no shipped GUI login or sync. Previous independent REST/OAuth module #948 and OS keyring #947 are already accepted.

## Scope and guarantees

- Bind a caller-provided, **registered** TCP port on IPv4 `127.0.0.1` before asking the host to open the system browser.
- Construct exact `http://127.0.0.1:<port>/akula/yandex/callback` in the same session as one-time PKCE verifier and 256-bit CSRF `state`.
- No automatic port selection in production: a port must be authorized in the app's Yandex OAuth Redirect URI configuration. Tests bind port 0 privately.
- Only one HTTP/1.1 GET, exact Host (no second Host), exact callback path, no request body/transfer encoding, no absolute URI target; 8-KiB header cap and 2-second socket deadline.
- Reject wrong state, denied consent, malformed callback, wrong host, unsolicited body, non-loopback connections. An unrelated `/favicon.ico` yields 404 without consuming the attempt. Limit 12 requests and a 120-second total wait.
- On success consume the code and verifier once, return only the existing `YandexTokenExchange` for **separate** HTTPS token exchange. The browser response never echoes code, state, token or account identity. Never record any of these in logs or settings.
- No OS token storage, no account binding, no Yandex Disk write, no native UI button, no startup listener and no sync authority. This is a library API; the native host must call it explicitly on a worker.

## Offline acceptance

```sh
cargo test --manifest-path modules/yandex-disk-rest/Cargo.toml
cargo check --manifest-path modules/yandex-disk-rest/Cargo.toml
```

Windows/Linux CI owner: `.github/workflows/yandex-disk-rest-isolated.yml`; real consent is out of scope without a registered Yandex OAuth application ID and test account.

## Next

Implement an opt-in native read-only connection orchestrator: explicit browser opening, callback worker, token exchange, Yandex ID confirmation, app-folder root list, then OS-keyring write after user confirms the exact account+Vault+profile. A successful login alone does **not** authorize Sync or local file changes.

References: https://yandex.ru/dev/id/doc/ru/register-auth and https://yandex.ru/dev/id/doc/ru/codes/code-url .