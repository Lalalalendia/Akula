# AK-YD-TOKEN-03: bounded, one-shot Yandex OAuth token exchange

**Status: opt-in isolated transport candidate; not connected to a real account and not released.**

This PR is stacked after #942 and #940. It implements the last network action of the Authorization Code + PKCE S256 handshake while preserving the previous no-write boundary.

## Implementation

- Only a **validated** \`YandexTokenExchange\` produced from the bound callback can make this network request; it is consumed by the operation.
- One POST to the **fixed** \`https://oauth.yandex.com/token\` with the URL-encoded form. No arbitrary endpoint, cookies, client secret, redirect following, automatic retry or user-controlled HTTP URL.
- 5-second connect and 15-second read/write timeouts, max 16 KiB response. Fail-closed typed errors for invalid/expired grant, invalid OAuth app, rejected scope, rate limit, server failure, unexpected status and malformed reply.
- Parse only a bearer \`access_token\` with a positive \`expires_in\`. If \`scope\` is present, it MUST include \`cloud_api:disk.app_folder\`; otherwise fail closed. The Yandex OAuth endpoint can omit scope on full grant; a separate read-only API probe is still required before declaring access confirmed.
- Both access and optional refresh tokens exist only in \`Zeroizing<String>\` values and are exposed via synchronous borrowing closures. No Clone, Debug, Display, Serialize, SQLite, plaintext config, env-var fallback or native storage.
- No retry: a lost response may have consumed the code, requiring the user to restart authorization.
- Offline tests: granted/reduced/omitted scopes, invalid token types/encoding, missing/invalid expiry, oversized payloads, structured HTTP error mapping. No live user tokens or live service contacted.

## Not done

No native browser opener/loopback callback receiver; no registered OAuth app client ID; no stored tokens, no refresh rotation, no account identity binding, no user-facing settings, no remote file mutations, no local file operations, no automatic Sync and no installer feature changes.

## CI

\`\`\`sh
cd aquilum-app
cargo test --locked -p aquilum-core --features sync-yandex-disk-rest yandex_disk_rest
cargo check --locked -p aquilum-core --features sync-yandex-disk-rest
\`\`\`

Windows/Linux CI uses the existing isolated workflow; general Akula Main PR gate must also succeed before declaring acceptance.

References: [official Yandex authorization-code token exchange](https://yandex.ru/dev/id/doc/ru/codes/code-url), [RFC 7636 PKCE](https://www.rfc-editor.org/rfc/rfc7636), [RFC 8252 native apps](https://www.rfc-editor.org/rfc/rfc8252).
