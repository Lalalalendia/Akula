# AK-YD-KEYRING-05 — isolated Yandex Disk OS credential vault

Independent module based on the already established Akula Dropbox secret-store pattern; no native host wiring or OAuth registration in this PR.

- New standalone crate: modules/yandex-disk-secret-store with local [workspace], default no features.
- YandexRefreshToken is zeroizing, redacted in Debug, non-serializable, no Clone. No environment-variable, in-memory or plaintext-file fallback in production.
- YandexCredentialScope fingerprints the verified Yandex user ID, exact OAuth app ID, Vault UUID and profile ID, prefixed with an explicit domain tag. The OS key name contains no raw account or Vault identifier.
- Real Windows credential manager / Linux Secret Service via existing pinned keyring 3.6.3 backend when system-store feature is explicitly enabled.
- Save only reports success after exact read-back using constant-time comparison; errors fail closed, absence is not an empty token.
- Local forget does not revoke the token remotely; separate UI control and network revoke require their own design.
- Mock keyring backend is only compiled in cfg(test). Offline tests cover wrong account, client ID, Vault/profile, token rotation, readback mismatch, denial, invalid scopes and log redaction.
- Dedicated Ubuntu/Windows CI compiles default and real OS-keyring flavors; it does not write to OS keychains during test runs.

Boundary: no integration in normal installed Akula, no browser callback, no access-token persistence, no live OAuth request, no remote writes, no local File Gate mutation and no Sync activation.

Dependency: use only after the Yandex ID check introduced by #945 confirms id and client_id. #945 remains a separate Draft. This crate does not itself prove that its input identity strings came from the server.

Verification:
```sh
cargo test --manifest-path modules/yandex-disk-secret-store/Cargo.toml --no-default-features
cargo check --manifest-path modules/yandex-disk-secret-store/Cargo.toml --features system-store
```

Reference: modules/dropbox-secret-store, https://yandex.ru/dev/id/doc/ru/user-information