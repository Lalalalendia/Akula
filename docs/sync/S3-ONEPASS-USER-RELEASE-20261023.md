# Akula S3: one-pass user release branch (2026-10-23)

## Active distribution policy — one installed Akula, one signed update channel

Owner decision 2026-10-23 supersedes the former separate S3 Beta / Stable packaging plan.
The two users (owner and a friend) keep ONE installed Akula each. New versions arrive
through the existing Akula-owned, Minisign-verified Windows updater and install over
the prior Akula installation. No S3-only installer, parallel installation, or beta-to-stable
promotion is required. The `s3-pair-beta` Cargo feature is an internal capability gate
compiled into the ordinary signed Windows binary; actual S3 operations remain explicit
user actions, not unattended background synchronization.

Tests use disposable notes and a disposable scoped S3 bucket, not valuable user data.
This reduces the scope of pre-release product-hardening, but does NOT permit silent
overwrite/delete, bad credential isolation, false `Synced`/ACK, or signed-update bypass.
Automated compilation, exact-head CI, guarded new-file File Gate receiving and NSIS GUI
smoke come before publication. The real two-PC Yandex scenario happens on that same
published version; any defect is corrected by the next ordinary signed Akula update.



**Canonical product UX**: [Notion AKULA-S3-PAIR-UX-01](https://app.notion.com/p/40132a84beec818b8672e88facc489f4).
**Source**: current `main@a2875a19f173d8a8bfec21e7106d5bdc92fbf722`.
**Target branch**: `sync/s3-one-pass-user-flow-20261023`.

## Workflow change

One integration branch, one eventual reviewable PR, **one initial complete implementation before full CI**, then repeat `test -> fix -> test` on the same branch. Small commits are welcome solely for reliable rollback/bisect; they are **not** acceptance steps and do not trigger separate PRs for every internal module.

A lightweight syntax/format check while coding is allowed; deferring elementary compiler errors is not a product milestone. Full automated Core/Native/MinIO E2E CI is run after the complete end-to-end code is written and again on the exact integration SHA. Physical two-installed-Akula Yandex acceptance takes place on the next ordinary signed Akula update with S3 included, not as a prerequisite for creating that update. No PR is marked ready based on an isolated component test.

## Locked v0 behavior: two Windows installations + Yandex S3 over the internet

1. A: open one native Sync page, select Yandex preset, enter bucket and protected credentials, verify read **and write-readback** access.
2. A: bootstrap/confirm a unique remote VaultId and authentic, pinned remote origin; create a single encrypted password-protected `.akula-sync` bundle that contains only the selected cloud profile, the scoped credentials and remote trust identity.
3. B: import bundle + password into a **new empty** local workspace; credentials enter the local OS protected store, not JSON, logs or clipboard. Never ask B to retype bucket, region, endpoint or access key.
4. A makes a new Markdown note; background worker uses existing trusted preflight, immutable S3 blob/revision/manifest, journal and provider CAS.
5. B verifies the pinned remote head and data, then uses existing OS-hosted File Gate **create-if-absent**, opens the note in Native UI; cold restart confirms matching bytes/hash.
6. B adds another **new** note and A receives it. Show actual progress and `Received` / `Needs review`; never show durable `Fully synced` without independently proved native receipt and baseline.
7. Repeated import is idempotent and checks binding; collisions, changed credentials, unknown remote root, network/SDK failures, ambiguous publication, changed editor file, unknown manifest or invalid invite always refuse or HOLD.

**V0 excludes** overwrite of existing local files, deletion propagation, automatic merge of diverged content, uncontrolled retry of CAS/ACK, and per-device credential revocation from shared scoped keys. Existing G1–G6 hardening does not block **unconfirmed new-file delivery**, but an eventual full durable Sync mode needs it.

## Code to finish BEFORE starting the full integration test loop

- **Native**: compact Sync settings UX, structured `SyncAction` commands and `SyncStatus`, background worker, secure file/password dialogs, WinCred scoped profile import/export, lifecycle cancellation; compile `s3-pair-beta` into the single signed Windows Akula executable; the user explicitly activates it from Settings, and receiving uses the already implemented cooperative per-user File Gate custody scope.
- **Core**: explicit (not ambient AWS profile) credentials for the S3 SDK worker, bounded authenticated read+write connection probe, conditional and authenticated genesis bootstrap, protected import/export envelope (Argon2id/AEAD with fixed limits), app-data profile binding and non-secret local state.
- **Sync**: one host-level Rust `sync_once` that uses existing `S3HeadJournal::publish_head_cas` and `receive_single_verified_s3_additive_once` with bounded backoff; create-new only, no replacement. Versioned status events; never mint confirmed ACK from `UnconfirmedLocalCreate`.
- **Tests**: complete integrated tests written with code, but full CI run only after all four blocks above compile as an executable user path. Tests include standalone Windows Native build, independent-process MinIO, corrupt invite, credentials rotation, foreign remote VaultId, S3 CAS loss, duplicate path, restart, offline, exact BLAKE3, and post-update private Yandex/two-Windows-installed manual acceptance.

## Test-fix loop (repeat, no new "phase" per failure)

1. Lock exact branch HEAD, run Rustfmt/Clippy, Windows and Linux Native/installer, full required CI and MinIO isolated bucket E2E.
2. Capture **all red jobs and failure classes** (compiler/ABI, architecture ratchet, API/security, scenario mismatch, environment).
3. Fix them in one or more commits on this **same branch**. No new peripheral project, no green by skipping tests or granting broader storage authority.
4. Re-run entire applicable suite on latest HEAD. A partial green lane does **not** close the user goal.
5. Once mandatory automated checks and the cooperative Native File Gate path pass, merge to main and publish the **next ordinary signed Akula Windows release** with the S3 feature compiled in. Preserve safe HOLD behavior and the ability to reinstall the previous signed release. This is NOT a second installer or channel.
6. Both users update their existing Akula installation and run A→S3→B and B→S3→A with a private disposable Yandex bucket. Verify restarts, wrong passwords, revoked keys, offline cases, collisions and exact bytes. If anything fails, fix it and publish the next ordinary signed version; mark the S3 scenario unverified until observed.

## Two acceptance checkpoints for ONE signed release

**Checkpoint 1: pre-publication automated validation.** The ordinary signed Windows build includes S3 and is installed over Akula. Physical two-PC testing is deferred until that signed update exists; experimental S3 functionality is not claimed fully verified.


- [ ] Setup performs a real limited read/write-readback proof and uses only credentials from WinCred.
- [ ] Exported `.akula-sync` is password-authenticated and encrypted; wrong password and tamper fail before any import mutation.
- [ ] B imports once without entering S3 parameters and registers a distinct local workspace linked to the same trusted cloud identity.
- [ ] Automated isolated MinIO A→B and B→A verifies exact bytes, encrypted import, restart and no-replace collision behavior; use the real installed Akula signed update for subsequent manual acceptance.
- [ ] Native status is accurate when a file is only staged, published locally-but-unconfirmed, or RequiresReview.
- [ ] Required signed Windows installer, Core/MinIO, Application/security and architecture checks PASS on one exact SHA after latest main integration.
- [ ] Native on Windows uses the explicitly provisioned OS-known per-user cooperative File Gate lock, passes `Some(&scope)` and refuses forged/missing scopes. This is **not installer-attested hostile-user protection**, but it must work for new-file-only receiving before the next update.
- [ ] Publish one signed Akula installer with S3 explicitly user-invoked, new-file-only and marked experimental in its settings. Use disposable test vaults, limited-scope keys and prior signed-version recovery instructions.

**Checkpoint 2: user acceptance on the same installed Akula version, not a channel promotion.**

- [ ] Two physically distinct Windows Akula installations updated through the normal signed channel, private disposable Yandex S3: setup A, encrypted export, import B with no second key entry, new file A→B and B→A; exact byte/BLAKE3 match after cold restart.
- [ ] Negative cases: invalid password, tampered bundle, foreign remote scope, revoked key, offline, path collision, second import; no silent overwrite/delete or false confirmed ACK.
- [ ] Record the exact signed version and results. On failure, ship the next signed Akula update. Until both-PC acceptance PASS, describe the S3 user scenario as **unverified**, without restricting unrelated Akula functions.

## Initial branch content

The first commit imports the existing **36-file** #428 Windows Credential Manager/read-only probe prototype on top of fresh main, reconciling `Cargo.lock`, `native/Cargo.toml` and EN/RU locales. This is a **foundation, not the end-to-end implementation**. No production automatic S3 flow has been enabled, and the branch should not be treated as ready to merge.

## Release semantics note

Publishing the ordinary Akula update makes S3 available to two test users; it does not prove full synchronization reliability. The former `None` receiver scope was replaced with OS-known cooperative custody and `Some(&scope)` in PR #649, but Windows installed acceptance and exact-head CI still have to confirm the complete path. No silent overwrite/delete, false confirmed ACK or credential leakage is permitted.
