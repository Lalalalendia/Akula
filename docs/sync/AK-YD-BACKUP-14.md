# AK-YD-BACKUP-14 — encrypted immutable one-object backup laboratory

**Status:** offline-tested, non-installed backup **building block**. NOT a live Yandex Disk upload, not full backup of a workspace, and not two-way sync.

## Rationale

The [official Yandex Disk REST upload API](https://yandex.com/dev/disk-api/doc/en/reference/upload) requires two requests: obtain a short-lived upload link (GET with `overwrite=false`) and PUT bytes to the linked URL. PUT may answer `201 Created` or `202 Accepted` when the file is not yet moved to Disk. Neither status can be treated as a verified backup. App-folder listings are not consistent snapshots; do not use them to infer deletion or import data automatically.

## Contracts

- **One bounded archive object**, not a directory sync. In-memory plaintext at most 8 MiB (newer code must chunk a real vault and provide a versioned encrypted manifest).
- **Encryption before remote transfer:** independent 256-bit recovery key; AES-256-GCM with a freshly generated 96-bit nonce. Random 128-bit name `app:/akula-backup-v1-<32 lowercase hex>.akulaenc` in the app folder root (avoids remote mkdir and path traversal). Entire app path is authenticated additional data, so moving bytes to a different backup path fails decryption.
- Recovery key is never embedded in the archive, remote path, application config or logs. A real UX needs a deliberately designed recovery-key export/confirmation flow before uploads.
- `SealedBackup::create_once_and_verify` accepts only **create-only** transports that refuse pre-existing object IDs. It never overwrites or deletes objects and cannot publish a Sync baseline. The HTTP adapter (NOT present here) must set `overwrite=false`, reject arbitrary signed-upload URLs/redirects, and avoid retries of ambiguous writes.
- A 201/202/lost-response hint is NOT enough: a separate bounded full-object readback is required, with SHA-256 comparison of the encrypted bytes. A 202-not-yet-visible backup remains `NotYetVerified`; only readback can be retried later, **never automatic duplicate PUT**.
- Remote bytes can be restored to **memory only**, after authenticated AES-GCM validation; no workspace file or editor buffer is modified.
- Unit tests exercise wrong key/path, tag/nonce tampering, max length, collision, 201/202 semantics, an uncertain committed/lost write, invisible object, denied write, oversized readback, corrupt remote content and explicit recovery key import.
- No live credentials, HTTP upload/download, File Gate, manifest pointer, scheduler, sync provider mapping, release or OS credential access.

## Acceptance

```sh
cargo test --manifest-path modules/yandex-disk-backup-lab/Cargo.toml
```

Separate exact-HEAD Windows/Linux isolated CI and required repository checks must pass. Next PR: opt-in native REST transport that obtains the uploader link from the **fixed** Yandex API origin, validates the returned Yandex-hosted HTTPS endpoint and runs a bounded remote readback, without enabling user-facing backup until a test account and recovery-key UX exist.
