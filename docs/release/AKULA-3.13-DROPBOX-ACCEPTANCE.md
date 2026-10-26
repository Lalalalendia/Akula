# Akula v0.3.13 — production release acceptance

**Product decision (23 October 2026):** Akula has no separate test distribution channel for this version. The release target is the ordinary public signed Akula release, with Dropbox read-only connection UI included in the standard installed binary. This document does not authorize bypassing signature, version, CI, or protected publication safeguards.

## Scope shipping to all users

- Existing desktop functionality and separate Akula package identity.
- Native Dropbox account setup and user-initiated OAuth PKCE, OS credential store, independent pinned HEAD verification, read-only reconnect and fail-closed error states (PRs #669 and #670).
- Dropbox read-only feature is a dependency of the ordinary `installed` feature. No special test executable or build feature is needed for end users.
- Uploads, deletes, automatic file application and **two-way sync remain disabled**. Their correctness and security have not been established; do not advertise them as implemented.
- Missing account configuration leaves the UI `DISCONNECTED / SYNC_DISABLED`. Ordinary local notes and other app functions remain usable without Dropbox.

## Public release verification

1. All canonical versions in `aquilum-app/native/Cargo.toml`, `aquilum-app/core/Cargo.toml`, `aquilum-app/Cargo.lock` and `aquilum-app/native/packaging/akula-windows.json` equal 0.3.13. Also align `akula-linux.json` for later Linux packaging, but do not silently imply that Linux signed publication is approved.
2. `node scripts/akula-version.cjs check` and standard Rust builds with `--locked --features installed` succeed from the exact PR HEAD, with Dropbox code compiled in by default.
3. Windows NSIS identity and installed GUI smoke (install → edit → save → restart → uninstall) succeed and preserve existing Akula user data and independent Aquilum installation.
4. Read-only Dropbox tests and native UI state tests succeed. Real-account Dropbox authorization, OS keyring/revoke and cold restart must be reported honestly as unverified if not performed; no claims of functioning sync.
5. Merge reviewed PR #670 after required CI, then publish from protected **main** using the existing `.github/workflows/akula-windows-signed-candidate.yml` workflow, `release_version=0.3.13`, `publish_beta=true`. Preserve approval via `akula-release` environment, detached Minisign signatures, independent verification and source/lock provenance. Do **not** reactivate retired `release.yml` or inherited Aquilum workflows.
6. Confirm GitHub Release `v0.3.13` is public and Latest with verified `Akula_0.3.13_x64-setup.exe`, its detached signature, `latest.json` and `latest.json.sig`; verify updater sees the signed version from an installed previous release.
7. Windows 7 compatibility remains a separate acceptance requirement; Windows 10/11 CI cannot prove it.
8. Linux is a normal supported build target, but **its public signed update/release channel is separately gated**. Publish Linux packages only through an Akula-owned verified signing and installed QA path, never through Freaction/Aquilum metadata.

## Outstanding work

- Required CI and signed Windows publisher run on exact main commit.
- Independent test of real Dropbox account/consent and installed Windows restart.
- Signed Linux release infrastructure and installed acceptance.
- Write/sync protocol and two-device conflicts if a later release is to offer complete bidirectional synchronization.

Sources: [PR #670](https://github.com/Lalalendia/Akula/pull/670), [Windows publisher workflow](https://github.com/Lalalendia/Akula/blob/main/.github/workflows/akula-windows-signed-candidate.yml), [Akula version contract](https://github.com/Lalalendia/Akula/blob/main/docs/release/AKULA-VERSIONING-01.md).


## Akula-owned Dropbox OAuth Console — status observed 24 October 2026

- Public client ID (safe to embed in source): `gzrirdrrx4tarly`. App is `Scoped App (App Folder)` with folder name `Akula`; **do not use the Aquilum client ID**.
- Exact OAuth PKCE callback configured in code: `http://127.0.0.1:47219/dropbox/callback`. The Dropbox Console screenshot shows the address in the input next to **Add**, not proof the **Add** button was pressed. Owner must register the URI and verify it appears in the saved redirect list.
- `account_info.read` is checked in the screenshot. `files.metadata.read` is **not** checked. `files.content.read` is outside the visible part of the screenshot, so its state is unknown. Owner must enable these two read-only scopes and save on the Permissions tab. No `files.*.write` or `account_info.write` required.
- Dropbox app remains in **Development** and the Development users list shows only the owner. Production authorization for arbitrary users is **not yet approved**. Complete Dropbox's review for public distribution before describing mass-market sign-in as ready.
- PR #670 includes an Akula-fixed, non-secret client ID and callback port in the ordinary `installed` build. Serialized settings ignore old QA app IDs/ports, and the two corresponding inputs were removed from the native UI. **Account-only sign-in is implemented:** the user can click `Войти в Dropbox` without a Vault ID, profile or HEAD pin; a successful OAuth PKCE flow proves account/root identity and stores a refresh token in the OS keyring. A separate `Проверить сохранённый вход` action checks the saved identity after restart without a browser. **These code paths have synthetic tests but no confirmed live Dropbox account acceptance.** A separately paired Vault still requires independently verified identifiers and pinned HEAD before read-only remote proof; no sync or local file application is enabled.
- Until verified live authorization, cold-restart OS secret recovery, and separate publisher trust are evidenced: `SYNC_DISABLED`, no local apply, no upload/delete and no assumption of verified user files.

Owner checklist (external Dropbox Console, cannot be completed from repository CI):

- [ ] Click **Add** for exact loopback redirect and verify saved entry.
- [ ] Enable and save `files.metadata.read` and `files.content.read`; keep only the three required read scopes.
- [ ] Verify real account-only browser PKCE, the exact approved read-only scopes, OS-keyring save, cold restart, manual saved-sign-in refresh, and local credential removal on a disposable test account.
- [ ] Separately verify advanced, independently pinned Vault read-only proof. Do not treat account-only sign-in as Vault trust or permission to sync.
- [ ] Apply for **Production** access before public user authorization.
