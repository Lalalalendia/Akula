# Akula S3: one-pass user release branch (2026-10-23)

**Canonical product UX**: [Notion AKULA-S3-PAIR-UX-01](https://app.notion.com/p/40132a84beec818b8672e88facc489f4).
**Source**: current `main@a2875a19f173d8a8bfec21e7106d5bdc92fbf722`.
**Target branch**: `sync/s3-one-pass-user-flow-20261023`.

## Workflow change

One integration branch, one eventual reviewable PR, **one initial complete implementation before full CI**, then repeat `test -> fix -> test` on the same branch. Small commits are welcome solely for reliable rollback/bisect; they are **not** acceptance steps and do not trigger separate PRs for every internal module.

A lightweight syntax/format check while coding is allowed; deferring elementary compiler errors is not a product milestone. Full platform and user E2E CI is run after the entire end-to-end flow has been written, and rerun on every final merged SHA. No PR is marked ready based on an isolated component test.

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

- **Native**: compact Sync settings UX, structured `SyncAction` commands and `SyncStatus`, background worker, secure file/password dialogs, WinCred scoped profile import/export, lifecycle cancellation; remove QA-only gating only when end-to-end safety is accepted.
- **Core**: explicit (not ambient AWS profile) credentials for the S3 SDK worker, bounded authenticated read+write connection probe, conditional and authenticated genesis bootstrap, protected import/export envelope (Argon2id/AEAD with fixed limits), app-data profile binding and non-secret local state.
- **Sync**: one host-level Rust `sync_once` that uses existing `S3HeadJournal::publish_head_cas` and `receive_single_verified_s3_additive_once` with bounded backoff; create-new only, no replacement. Versioned status events; never mint confirmed ACK from `UnconfirmedLocalCreate`.
- **Tests**: complete integrated tests written with code, but full CI run only after all four blocks above compile as an executable user path. Tests include standalone Windows Native build, independent-process MinIO, corrupt invite, credentials rotation, foreign remote VaultId, S3 CAS loss, duplicate path, restart, offline, exact BLAKE3, and final private Yandex/two-Windows-installed manual gate.

## Test-fix loop (repeat, no new "phase" per failure)

1. Lock exact branch HEAD, run Rustfmt/Clippy, Windows and Linux Native/installer, full required CI and MinIO isolated bucket E2E.
2. Capture **all red jobs and failure classes** (compiler/ABI, architecture ratchet, API/security, scenario mismatch, environment).
3. Fix them in one or more commits on this **same branch**. No new peripheral project, no green by skipping tests or granting broader storage authority.
4. Re-run entire applicable suite on latest HEAD. A partial green lane does **not** close the user goal.
5. Stage a real Windows package, execute A→S3→B and B→S3→A with clean installs and a user-owned private Yandex test bucket; record receipts and UI screenshots without exposing secrets. If physical PC access is absent, say explicitly that the last gate is **unverified**.

## Frozen release gates

- [ ] Setup performs a real limited read/write-readback proof and uses only credentials from WinCred.
- [ ] Exported `.akula-sync` is password-authenticated and encrypted; wrong password and tamper fail before any import mutation.
- [ ] B imports once without entering S3 parameters and registers a distinct local workspace linked to the same trusted cloud identity.
- [ ] A→B and B→A new Markdown files are visible via the **installed UI**, with exact bytes after cold restart; collision does not overwrite either original.
- [ ] Native status is accurate when a file is only staged, published locally-but-unconfirmed, or RequiresReview.
- [ ] All required CI and architecture ratchet checks PASS on one exact SHA after latest main integration.
- [ ] Two physically distinct installed Windows machines, private Yandex S3, offline/revoked credentials and restart PASS; without them, no "released" claim.

## Initial branch content

The first commit imports the existing **36-file** #428 Windows Credential Manager/read-only probe prototype on top of fresh main, reconciling `Cargo.lock`, `native/Cargo.toml` and EN/RU locales. This is a **foundation, not the end-to-end implementation**. No production automatic S3 flow has been enabled, and the branch should not be treated as ready to merge.
