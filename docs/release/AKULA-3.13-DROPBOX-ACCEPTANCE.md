# Akula 3.13 — Dropbox test integration

Status: **development preview, not a published release**. Version in `aquilum-app/native/Cargo.toml`: `0.3.13`. This document is the acceptance boundary, not evidence of a completed integration.

## Scope of the 3.13 experiment

- Reuse merged Dropbox T0/T1 modules, including read-only OAuth PKCE, registered loopback callback, OS-protected refresh token and explicit read-only restart verification (PR #669).
- Make these flows accessible through a deliberate user-facing integration in Akula. Do not mark complete until a real native UI action calls the real connection code, with no mock credentials or fake success.
- Use a separate test vault and Dropbox **App Folder**, no production notes. No startup auto-connect.
- Display explicit states: disconnected, authorizing, verified read-only / publisher not approved, revoked, unavailable OS keyring, wrong account/root/head, network error.
- Never display, log or persist authorization code, access token, refresh token or OAuth state outside the intended protected storage.
- **No uploads, deletes, file application, baseline promotion or two-way sync** until publisher signature, freshness/anti-rollback, conflict handling and CAS are separately implemented and tested.

## Required acceptance

1. Desktop Windows build compiles with actual Dropbox modules and the 3.13 version; Rust unit and security tests pass in CI.
2. A user explicitly starts OAuth in their system browser and grants only `account_info.read files.metadata.read files.content.read`.
3. A developer-owned pinned BLAKE3 of synthetic HEAD, account ID and root namespace are verified independently; a successful read stays `PUBLISHER_NOT_APPROVED / SYNC_DISABLED`.
4. OS Credential Manager stores refresh token, verified through a cold restart and read-only resume; revoke, missing/locked credential store, wrong account/root, wrong HEAD and changed permissions fail closed.
5. A real Dropbox account is used for manual Windows end-to-end testing; never put secrets in CI, screenshots, logs or Notion.
6. A Windows 7 acceptance is separate and cannot be inferred from modern Windows or Linux CI.
7. No release installer is published until Akula signing, installer identity, rollback and release gates are approved.

## Implementation work remaining

- Wire standalone `modules/dropbox-connect-lab` into the desktop workspace behind an explicit opt-in feature and native UI controls, without enabling writes.
- Add end-to-end UI state tests, including cancellation, timeout and restart; verify the native executable actually includes the connector.
- Provide safe test setup instructions and an isolated Windows beta artifact, respecting `RELEASE-LOCK-01`.
- Run and document live Dropbox acceptance; only then consider write/sync work in a later gated change.

Sources: [PR #669](https://github.com/Lalalalendia/Akula/pull/669), [Dropbox QA in Notion](https://app.notion.com/p/40232a84beec81be9e10d428547df0fb).
