# AK-YD-CONNECTOR-06 — isolated Yandex REST + OAuth transport

Supersedes the unfused, unmergeable shared-Core Cargo feature experiments in Draft #940, #942, #944 and #945 without removing their historical work.

Architecture: the provider adapter is not the Sync engine. The one true provider-neutral planner, journal, File Gate and RemoteStore remain in aquilum-core::sync. The adapter is standalone, opt-in, and MUST NOT be wired as a write authority until guarded provider semantics are demonstrated.

Source carried forward without functional changes:
- REST read-only app:/ listing, fixed HTTPS Yandex Disk API origin, bounded data, typed errors, observed disk:/ app root, unverified directory pages;
- Authorization Code OAuth with PKCE S256, one-use CSRF state and fixed redirect validation;
- single-use bounded token exchange via official fixed Yandex token endpoint, zeroizing access/refresh token buffers and rights checks;
- Yandex user ID + OAuth app identity validation via fixed login.yandex.ru/info endpoint.

The module is compiled and tested with its own manifest, no shared Core Cargo dependency or new native installation feature. It does not implement RemoteStore, upload, delete, local apply, sync scheduling, or file mutation. It has no registered OAuth app ID or native browser callback, and stores no secrets. Protected OS refresh token storage already exists independently in merged #947.

Direct CI owner: .github/workflows/yandex-disk-rest-isolated.yml (Windows + Linux). The owner is registered in ci-rust-scope.test.cjs. This avoids triggering unrelated shared-manifest root/VM tests without changing or weakening those tests. Any later Core/native bridge will have to pass the normal required ruleset.

Acceptance:
```sh
cargo test --manifest-path modules/yandex-disk-rest/Cargo.toml
cargo check --manifest-path modules/yandex-disk-rest/Cargo.toml
```

Next step after merge: safe opt-in native host bridge, explicit user-approved Yandex OAuth application registration, OS keyring binding to verified account, and read-only disposable account integration tests. No automatic write or two-client sync until CAS/immutable-object protocol is proven.

Reference: https://yandex.ru/dev/disk-api/doc/ru/ ; https://yandex.ru/dev/id/doc/ru/codes/code-url