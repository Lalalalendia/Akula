# AK-YD-IDENTITY-04 — read-only Yandex account and OAuth application binding

Status: stacked Draft after #944 → #942 → #940. No normal installer integration.

## Why

A successful OAuth token exchange does not prove that the user selected the
expected Yandex account. Account information must be checked before binding a
refresh token to one local Akula Vault/profile. Reusing a credential by display
name, login hint, S3 bucket or previous configuration is not safe.

The Yandex ID \`/info\` endpoint returns stable standard \`id\` and \`client_id\`
fields for OAuth tokens issued with scopes of **other Yandex services**, even
when no additional \`login:info\` permission was requested.

## Code

- \`YandexEphemeralTokens::verify_yandex_account(expected_client_id)\` sends
  one bounded \`GET https://login.yandex.ru/info?format=json\`, passing the
  access token only as an \`Authorization: OAuth\` header, never a query arg.
- Strict fixed HTTPS origin, zero redirects, explicit network timeouts, max
  8 KiB response, 401/403/429/5xx typed errors.
- Parse only standard \`id\` (decimal Yandex user identifier) and \`client_id\`.
  Unexpected OAuth client ID, missing/malformed data, or over-limit responses
  fail closed; no account is reported as verified.
- \`YandexAccountIdentity\` is a point-in-time read-only identity observation.
  It is not a Sync permission, remote snapshot, durable credential scope or a
  proof of access to the app folder.
- Offline tests use official-style JSON and mismatching/malformed responses.

## Next separate step

Require an OS-backed secret vault whose key is derived from the verified
\`id\` + \`client_id\` and an explicit local Vault/profile. The vault must
fail closed when OS keyring services are unavailable, read back persisted
refresh tokens for verification and never reuse them across identity changes.
A second read-only app-folder API probe must verify actual disk access.
Real browser callback, OAuth app registration and UI are still missing.

This module never stores secrets, writes to Yandex Disk, modifies local notes,
activates the sync engine, or changes the installed build.

Sources:
- https://yandex.ru/dev/id/doc/ru/user-information
- https://yandex.ru/dev/id/doc/ru/codes/code-url
