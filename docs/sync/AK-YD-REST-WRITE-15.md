# AK-YD-REST-WRITE-15 — fixed-origin immutable encrypted-object uploader

Status: **opt-in network library** only, not installed and not connected to native UI. Base requirement: merged #957 encrypted-object create-once/readback protocol. No live account test or production credential has been used.

## Transport contract
1. `YandexDiskRestBackupPort::new(&ephemeral_access_token)` takes one caller-owned short-lived OAuth access token after explicit account/App Folder verification. The transport never refreshes, stores, prompts for or logs tokens.
2. Upload metadata request: only GET `https://cloud-api.yandex.net/v1/disk/resources/upload`, with exact validated `app:/akula-backup-v1-*.akulaenc` path and **`overwrite=false`**. The network client has fixed origins and redirects **disabled**.
3. Parse Yandex `{"href","method":"PUT","templated":false}` but treat the returned href as untrusted. Only HTTPS 443 on real Yandex uploader hosts (`uploader*.dst.yandex.net`, `uploader*.disk.yandex.net`) and `/upload-target` may receive ciphertext; never send bearer token to the transient href.
4. Exactly one PUT of a bounded encrypted envelope. `201 Created` and `202 Accepted` remain unverified until a separate full readback. Unknown status or transport loss means `CommitUncertain`; **no automatic PUT retry**.
5. To read back, GET the fixed Yandex `/resources/download?path=app:/...` API, then follow only a validated signed GET link on `downloader.disk.yandex.ru`, `downloader.disk.yandex.net` or `*.storage.yandex.net`. Signed links are never exposed to logs and are not redirected to unknown hosts.
6. Bounded streaming response max 8 MiB plus encryption overhead. A missing resource returns no remote bytes, never a verified empty archive. The parent #957 protocol compares SHA-256 of *all* ciphertext bytes before producing a receipt and checks AES-GCM authentication before returning plaintext to memory.
7. No endpoint, host, operation or file path can be supplied by an untrusted Yandex metadata response except a validated allowlisted signed HTTPS URL. No remote delete, overwrite, update or listing-based trust.

## Limitations and non-goals
- This code adds **actual REST HTTP methods** behind an opt-in Cargo `network` feature, but **no real test account/authentication has been exercised**. Host allowlists intentionally fail closed if Yandex changes its signed URL host format.
- `overwrite=false` is an API request, not a proven cross-client compare-and-swap guarantee. A 202 may remain invisible; only a manual bounded readback retry is possible. Transport is unsuitable for bidirectional live Sync.
- No OS credentials are read, no secret copied to settings, no user-file ingestion or File Gate restore, no background worker, no recovery-key UI, no plan to handle >8 MiB or a full workspace, and no release/deploy.
- Follow-up: register Akula-owned OAuth app and run disposable-account 201/202/collision/lost-response/folder-scope E2E; design encrypted chunk manifest plus explicit recovery key export and restore-to-new-workspace gate.

## Acceptance
```sh
cargo test --manifest-path modules/yandex-disk-backup-lab/Cargo.toml
cargo test --manifest-path modules/yandex-disk-backup-lab/Cargo.toml --features network
```
Required Application checks and new Windows/Linux isolated CI at exact commit SHA.

Sources:
- Official upload semantics: https://yandex.com/dev/disk-api/doc/en/reference/upload
- Public Yandex Disk REST API: https://yandex.ru/dev/disk/rest/
