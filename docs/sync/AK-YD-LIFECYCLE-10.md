# AK-YD-LIFECYCLE-10 — OS token presence and verified local forgetting

Status: safety-only library slice; no GUI, no live OAuth, no background processing and no sync/remote access.

## Guarantees

- `YandexCredentialVault::inspect_local(scope)` returns **Missing** or **PresentUnverified** only. A populated system keyring is **not proof** that Yandex accepts the token, OAuth consent still exists, access to app:/ is allowed or a remote snapshot is complete.
- Malformed/corrupted stored refresh material returns **InvalidToken**, not a success status.
- `forget_local(scope)` requires a second OS read to prove the key was absent after delete; a misleading delete success or unavailable readback fails closed with a typed error.
- Exact account, client ID, local Workspace UUID and profile are still partitioned in a SHA-256 hashed native key name. No plaintext file fallback and no change to Sync Core.
- Local forgetting does **not revoke the OAuth grant on Yandex servers**. Documentation/user interface must provide a separate external revocation route and label it distinctly.
- The OS readback is a point-in-time check, not a cross-process lock or power-loss durability receipt. A concurrent login could recreate credentials later; this method does not establish any entitlement.

## Offline acceptance

```sh
cargo test --manifest-path modules/yandex-disk-secret-store/Cargo.toml --no-default-features
cargo check --manifest-path modules/yandex-disk-secret-store/Cargo.toml --features system-store
```

Unit tests inject misleading delete acknowledgements, failing OS reads, malformed token values and per-account scope boundaries. Windows/Linux `Yandex Disk OS secret-store offline checks` must pass. Do not merge before its exact-HEAD CI.
