# Akula S3: one-pass user release branch (2026-10-23)

**Canonical product UX**: [Notion AKULA-S3-PAIR-UX-01](https://app.notion.com/p/40132a84beec818b8672e88facc489f4).
**Source**: current `main@a2875a19f173d8a8bfec21e7106d5bdc92fbf722`.
**Target branch**: `sync/s3-one-pass-user-flow-20261023`.

## Workflow change

One integration branch, one eventual reviewable PR, **one initial complete implementation before full CI**, then repeat `test -> fix -> test` on the same branch. Small commits are welcome solely for reliable rollback/bisect; they are **not** acceptance steps and do not trigger separate PRs for every internal module.

A lightweight syntax/format check while coding is allowed; deferring elementary compiler errors is not a product milestone. Full automated Core/Native/MinIO E2E CI is run after the complete end-to-end code is written and again on the exact integration SHA. Physical two-installed-Akula Yandex acceptance takes place on the first published, installable S3 Sync Beta artifact, not as a prerequisite for building that beta. No PR is marked ready based on an isolated component test.

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

- **Native**: compact Sync settings UX, structured `SyncAction` commands and `SyncStatus`, background worker, secure file/password dialogs, WinCred scoped profile import/export, lifecycle cancellation; enable `s3-pair-beta` in a clearly identified opt-in beta package only after CI passes and the Native host supplies a trusted File Gate custody scope; leave stable/default installed builds unchanged.
- **Core**: explicit (not ambient AWS profile) credentials for the S3 SDK worker, bounded authenticated read+write connection probe, conditional and authenticated genesis bootstrap, protected import/export envelope (Argon2id/AEAD with fixed limits), app-data profile binding and non-secret local state.
- **Sync**: one host-level Rust `sync_once` that uses existing `S3HeadJournal::publish_head_cas` and `receive_single_verified_s3_additive_once` with bounded backoff; create-new only, no replacement. Versioned status events; never mint confirmed ACK from `UnconfirmedLocalCreate`.
- **Tests**: complete integrated tests written with code, but full CI run only after all four blocks above compile as an executable user path. Tests include standalone Windows Native build, independent-process MinIO, corrupt invite, credentials rotation, foreign remote VaultId, S3 CAS loss, duplicate path, restart, offline, exact BLAKE3, and post-package private Yandex/two-Windows-installed manual beta acceptance.

## Test-fix loop (repeat, no new "phase" per failure)

1. Lock exact branch HEAD, run Rustfmt/Clippy, Windows and Linux Native/installer, full required CI and MinIO isolated bucket E2E.
2. Capture **all red jobs and failure classes** (compiler/ABI, architecture ratchet, API/security, scenario mismatch, environment).
3. Fix them in one or more commits on this **same branch**. No new peripheral project, no green by skipping tests or granting broader storage authority.
4. Re-run entire applicable suite on latest HEAD. A partial green lane does **not** close the user goal.
5. Once the mandatory automated checks and trusted Native File Gate installation path pass, **publish an installable, opt-in `S3 Sync Beta` build** with rollback and safe HOLD behavior. Physical two-PC validation is deliberately deferred until this beta artifact exists; do not claim it has happened.
6. Install that exact beta on two distinct Windows PCs, run A→S3→B and B→S3→A with a private disposable Yandex bucket; verify restart, wrong password, revoked keys, offline, collisions and no data loss. If anything fails, fix the same product flow and publish a corrected beta. Do not promote the beta to stable without this acceptance.

## Two-stage release gates (decision 2026-10-23)

**Stage 1: pre-publication technical gate for a real installable opt-in S3 Sync Beta.** The user will run the first physical two-Akula test *in this released beta* rather than delaying packaging while waiting for two-PC results. This is not the same as declaring full stable Sync ready.


- [ ] Setup performs a real limited read/write-readback proof and uses only credentials from WinCred.
- [ ] Exported `.akula-sync` is password-authenticated and encrypted; wrong password and tamper fail before any import mutation.
- [ ] B imports once without entering S3 parameters and registers a distinct local workspace linked to the same trusted cloud identity.
- [ ] Automated isolated MinIO A→B and B→A verifies exact bytes, encrypted import, restart and no-replace collision behavior; use a real Native installed beta package for subsequent manual acceptance.
- [ ] Native status is accurate when a file is only staged, published locally-but-unconfirmed, or RequiresReview.
- [ ] All required Windows beta/installer, Core/MinIO, Application/security and architecture ratchet checks PASS on one exact SHA after latest main integration.
- [ ] Native beta actually passes a trusted installer-provisioned File Gate custody scope to its receiver; no `None` bypass or fake `Synced` badge. This is a **pre-beta** blocker, not a post-beta discovery.
- [ ] Publishing scope is an opt-in, clearly labeled experimental beta; stable/default build remains unchanged. User test vault, limited-scope keys, recoverable configuration and rollback instructions are available.

**Stage 2: post-publication beta acceptance before stable promotion.**

- [ ] Two physically distinct installed Windows Akula beta clients, private disposable Yandex S3: setup A, encrypted export, import B with no second key entry, new file A→B and B→A; exact byte/BLAKE3 match after cold restart.
- [ ] Negative cases: invalid password, tampered bundle, foreign remote scope, revoked key, offline, path collision, second import; no silent overwrite/delete or false confirmed ACK.
- [ ] Record the tested beta artifact/version and results. On failure, ship a corrected beta (not stable). Until both-PC acceptance PASS, release status is **Beta / user acceptance pending**; not `S3 Sync stable`.

## Initial branch content

The first commit imports the existing **36-file** #428 Windows Credential Manager/read-only probe prototype on top of fresh main, reconciling `Cargo.lock`, `native/Cargo.toml` and EN/RU locales. This is a **foundation, not the end-to-end implementation**. No production automatic S3 flow has been enabled, and the branch should not be treated as ready to merge.

## Release semantics note

Distribution of an installation artifact for user-led testing is not equivalent to verified production readiness. Specifically, the Native receive path still passing `None` for OS custody must be corrected *before* any beta purporting to receive files. The earlier manual two-PC pre-merge hold is superseded by the two-stage gates above; compile, security, cryptographic integrity, automation, crash-safe no-replace and honest status checks are **not** waived.
