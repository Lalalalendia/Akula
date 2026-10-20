# YC-S3-T01c — bounded Yandex S3 read-only authorization probe (Core slice)

Scope: opt-in `aquilum-core/sync-s3` only. No native UI enablement, no credential-store read in this slice, no live Yandex account used, no S3 writers or trusted Vault binding.

## Implementation contract

- Public Core API `probe_yandex_s3_read_only(bucket, access_key_id, secret_access_key)`; caller must supply credentials obtained from a protected source. Do not pass values through AppConfig, notes, clipboard, URLs, logs or issue reports.
- Explicit fixed `https://storage.yandexcloud.net`, region `ru-central1`, path-style addressing. No caller-chosen URLs; explicit credential provider avoids falling back to a different AWS profile.
- One signed `ListObjectsV2` call, prefix `akula-sync/`, `max_keys=1`. Never returns/records object identifiers or payloads, never writes an object. 4s connection timeout, 12s operation timeout, one attempt.
- Typed status: missing/bad-format keys, rejected credentials, access denied, missing bucket, clock skew, wrong region, timeout, network/TLS dispatch failure, unclassified service failure. Failure is never converted to success. A 403 response alone cannot reliably distinguish a non-existent bucket from denied access.
- A successful response proves **only** current ability to list the configured bucket under the selected permissions. It does NOT prove a particular Vault owner, writable credentials, bucket versioning, provider CAS, remote content integrity, safe Sync, or durable local File Gate authority.
- SDK request signing uses official AWS Rust SDK. Credentials may be copied into SDK memory during the request: clearing caller buffers cannot guarantee wiping SDK or allocator copies.

## Deliberate hold

1. Build/test Core `sync-s3` on Linux and Windows with `--locked`; verify independent native release build remains without the feature. Tests in this PR are pure local classification/admission tests.
2. Physically accept the masked Windows CredUI Save/Cancel/Replace/Delete flow from [#388](https://github.com/Lalalalendia/Akula/pull/388). Do not merge #388 based only on synthetic CredWrite tests.
3. Separate PR: invoke Core probe on a bounded background task through the approved native credential-provider interface. Do NOT call network from the settings click/render thread; do not extend credential lifetime unnecessarily; show only sanitized status; clear cached success on bucket/key rotation.
4. With user-approved throwaway scoped credentials, live Yandex test: empty bucket 200, normal private bucket 200, wrong key, denied ListBucket, absent bucket, revoked keys, no network, timeout and TLS failure. Record only sanitized outcome and request-count evidence.
5. Separate trust-binding and two-device/custody gates before ANY remote writes or sync.

No production credential or live Yandex permission has been exercised by this commit.
