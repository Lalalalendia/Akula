# AK-YD-NATIVE-13 — saved public account + local lifecycle UI

Stage: separate, opt-in desktop preview. It depends on AK-YD-PUBLIC-11 (#953) and merged AK-YD-LOCAL-12 (#954). No user-file sync or release enablement.

## User workflow
1. Open Settings → Sync → Personal Yandex Disk (REST, distinct from Yandex Cloud S3). For a test build, the public product OAuth client ID and exact redirect port must already be compiled; without them, login and local credential buttons are disabled.
2. Login uses system browser, PKCE and Yandex account+app:/ confirmation already accepted in #951. When the user clicks the separate **Confirm** button, Akula first writes a strictly validated **public-only** pointer to existing atomic settings.json: Yandex ID, product OAuth app ID, device-local workspace UUID, profile. A conflicting account requires explicit previous sign-out, never silent replacement.
3. Only if the public settings write succeeds may the existing worker persist refresh token into the system credential store; an error does not claim connected.
4. Following a cold restart, Settings displays the public Yandex ID with label **UNVERIFIED**. It does not automatically read secret data, refresh OAuth, open a browser or start Sync.
5. **Inspect local token** explicitly calls only the OS keyring and returns Missing/PresentUnverified. Neither state proves that Yandex currently authorizes the token.
6. **Forget on this device** explicitly deletes only the exact account/app/workspace/profile keyring entry, requires verified absence (already accepted #952), then removes that exact public settings pointer. A failed OS delete leaves the public pointer intact. A failed settings write reports failure and allows retry.
7. Workspace changes invalidate outstanding callback and local keyring events; no cross-workspace UI promotion.

## Safety limits
- No token, PKCE state, access key, client secret or authorization code is written to settings or workspace.
- No automatic reauthorization or refresh. Official Yandex OAuth refresh/revoke endpoints require `client_secret`; never embed it in a distributed desktop client.
- The user can follow the official Yandex ID access-management documentation to revoke remotely. Local forgetting does NOT revoke Yandex OAuth grants.
- No cloud file upload/delete, local File Gate write, journal publication or automatic Sync. All UI remains `SYNC_DISABLED`, and feature is excluded from installed/release builds.

## Acceptance
```sh
cd aquilum-app
cargo check --locked -p aquilum-native --features installed,yandex-disk-readonly-preview
cargo test --locked -p aquilum-native --bin aquilum-native --features installed,yandex-disk-readonly-preview yandex_disk -- --test-threads=1
```
Required Application checks and native Windows/Linux. This PR carries the exact pending public Core model from #953 for early CI only and MUST NOT be merged before #953 succeeds and is integrated. No deploy.
