# YC-S3-T01c-WIN-BRIDGE — *feature-gated*, read-only Windows connection preview

This is a stacked Draft integration preview based on the exact #388 branch.
#408 is independently merged to main, but this older #388 base requires the
Core probe module to be copied into the preview branch for the compiler.
The #423 background coordinator is separately reviewed and copied as an
inert standalone component. Deduplicate these integrations when restacking.

## Default release safety

- `aquilum-native --features installed` is unchanged in the **sense of authorization**: no SDK-enabled native connection path is compiled and no Check Connection button becomes active.
- Only `aquilum-native --features installed,yandex-s3-connection-preview` compiles the opt-in AWS SDK and enables the manual-only, read-only connection check.
- The UI remains explicit that **Sync is off**. A successful LIST means only that the provided principal could list that bucket/prefix at the time of the request. No Vault ownership, bucket-write grant, CAS, file mutation, P2P authorization, background scheduling, upload, sync or delete is implied.
- The user never pastes key material into native TextEditor. The credential pair is read in the background worker through `with_stored_keys` from Windows Credential Manager, scoped to `Akula/YandexObjectStorage/v1/<bucket>`; the caller receives only a fixed enum result. On other platforms, the UI remains disabled.
- SDK copies of secrets may persist in process memory. The vault's `Zeroizing` cleanup does not prove total erasure of all transitive SDK copies.

## Request lifecycle

- A Check Connection button appears **enabled only in the preview feature**, and only with a valid configured bucket, a locally present saved key and no current worker.
- Clicking creates a bounded single-flight OS thread using `ProbeSession`. The worker reads WinCred keys, starts an in-thread Tokio runtime, and performs exactly one bounded signed `ListObjectsV2` through `probe_yandex_s3_read_only` with fixed Yandex endpoint and `max-keys=1`.
- Only a fixed category is returned through mpsc and winit wake; never object keys, secret strings, raw SDK errors, server bodies or signed URLs.
- Changing bucket, successfully storing/replacing/deleting keys, leaving the Sync settings tab, closing Settings or reopening clears any remembered result, rejecting the previous request ticket.
- Invalidated in-flight requests still occupy the session's one worker slot until completion. This prevents a second request from simultaneously racing the first, but does not cancel the HTTP transaction already on the wire.
- After a successful LIST, the background worker re-reads the exact same Windows credential target and checks that the pair is unchanged. An external overwrite or deletion **during the network request** invalidates the success, proven by synthetic WinCred test. This is a point-in-time check and does not prevent a later edit/revocation or ABA rotation (old bytes restored); it is not durable remote Vault trust.

## CI and release acceptance

- [ ] Windows: exact-HEAD `--locked` default installed build, plus opt-in preview build, synthetic WinCred key read and rejection, stale-result tests, UI disabled conditions, localized messages, Core taxonomy.
- [ ] Windows physical CredUI: masked Save, Cancel, Replace, per-bucket separation, restart, local Delete. Synthetic CI alone does not prove these interactions.
- [ ] Real Yandex: **user-authorized, disposable private test bucket** with minimum `ListBucket` permissions, observe success, 403, 404, revoked key, offline/timeout and TLS. Do not ask for keys in GitHub, Notion or chat.
- [ ] Review changes in the original #388, #408 and #423 against upstream main to avoid introducing hidden detached privilege or stale config.
- [ ] Verify WinCred revalidation after LIST under a concurrent external replacement during a fake/isolated worker run; local sequential overwrite/delete synthetic cases already covered. Document that server-side revocation after the check is not observable without a new explicit request.
- [ ] No production release enabling the preview feature or automatic Sync until all human and live-provider gates pass.

PR dependencies: [Windows key store #388](https://github.com/Lalalalendia/Akula/pull/388), [Core read-only #408](https://github.com/Lalalalendia/Akula/pull/408), [background session #423](https://github.com/Lalalalendia/Akula/pull/423). Project record: [Notion Yandex S3](https://app.notion.com/p/3fe32a84beec8118a2b4e31f64434602).
