# AK-YD-OAUTH-02: one-shot PKCE authorization handshake (offline contracts)

**Status: stacked Draft development, NOT login support or cloud synchronization.**

This incremental change is based on AK-YD-REST-01 / PR #940. It is isolated behind the existing non-shipping \`sync-yandex-disk-rest\` feature.

## What is implemented

- Fixed \`https://oauth.yandex.com/authorize\` and \`https://oauth.yandex.com/token\` endpoints.
- CSPRNG-generated 256-bit PKCE verifier and an independent 256-bit CSRF state.
- S256 challenge with base64url encoding; RFC 7636 reference-vector regression.
- A registered redirect URI must be supplied explicitly: HTTPS with no userinfo/query/fragment, or an explicit 127.0.0.1 HTTP port.
- The pending authorization is one-use, in-memory only and expires after 10 minutes.
- Callback URI must match the exact protocol/host/port/path; reject wrong state, duplicates, malformed parameters, denied access, missing or invalid codes.
- On success, a temporary token-exchange request can be built using \`grant_type=authorization_code\`, the app's public client_id, code and code_verifier. The client secret is NEVER embedded.
- One-time verifier, state, authorization code and form body are wrapped in \`Zeroizing<String>\`; no derived \`Debug\` / serialization for sensitive structures.

## Explicitly not implemented

- No production Yandex OAuth application ID has been selected or registered.
- No system browser is launched, no listening loopback HTTP server, no OAuth callback routing in native UI.
- No POST to token endpoint, no access/refresh tokens, no token persistence, refresh or account identity binding.
- No remote file writes, local apply, activation of \`RemoteStore\`, or scheduled sync.
- The normal Akula installed binary remains unchanged and does not include this feature.

## Acceptance tests

\`\`\`sh
cd aquilum-app
cargo test --locked -p aquilum-core --features sync-yandex-disk-rest yandex_disk_rest
cargo check --locked -p aquilum-core --features sync-yandex-disk-rest
\`\`\`

Windows and Linux GitHub Actions run the same Rust commands. All tests are offline and use fabricated OAuth callbacks, not live tokens.

## Next required slice

Register/configure a Yandex OAuth native app with correct \`cloud_api:disk.app_folder\` scope and exactly matched redirect URI. Implement an OS-backed callback receiver, fixed-endpoint token exchange with strict HTTP error handling and expiring refresh tokens held in an OS keystore; add real user-initiated read-only test on a disposable account. Do not attach token exchange to \`RemoteStore\` write authorities.

Sources:
- https://yandex.ru/dev/id/doc/en/codes/code-url
- https://www.rfc-editor.org/rfc/rfc7636
- https://www.rfc-editor.org/rfc/rfc8252
