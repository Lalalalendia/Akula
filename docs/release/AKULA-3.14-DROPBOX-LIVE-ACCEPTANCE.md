# Akula Dropbox OAuth — owner-only Windows acceptance

## Separate the released build from the candidate

**Public signed v0.3.14 is a known failing baseline.** Its Windows OAuth launcher uses `explorer.exe`, which can open File Explorer instead of a browser. The public installer is immutable. Login, stored-token recovery, and Dropbox approval have NOT been demonstrated by that release.

**Candidate**: branch `fix/dropbox-account-relogin-scope-20261024` at `290606a665f76c49585dabd88411bd5bf190ed28`. Source changes use the system HTTPS handler via `ShellExecuteW`, initialize STA COM on the worker, and reject re-consent for a different saved account/root before storing credentials. Candidate is neither CI-accepted nor publicly released.

Both builds are account-only / read-only; Dropbox sync, writes, deletes, and trusted publisher approval remain disabled. `SYNC_DISABLED` must remain true after successful account login.

## A. Reproduce published 0.3.14 (do not mark PASS)

1. On a disposable Windows test profile install ordinary signed v0.3.14, open Settings → Cloud → Dropbox, click `Войти в Dropbox`.
2. Record whether File Explorer, a browser or neither opens. `WAITING_FOR_DROPBOX_ACCOUNT_CONSENT / SYNC_DISABLED` without an actual browser is **not** OAuth acceptance.
3. Do not infer successful consent or refresh-token storage. Do not paste OAuth callback URLs or secrets into reports.

## B. Owner-only Dropbox App Console prerequisite

Use [Dropbox App Console](https://www.dropbox.com/developers/apps), app `gzrirdrrx4tarly`, scoped App Folder `Akula`. Verify the saved callback `http://127.0.0.1:47219/dropbox/callback` (press **Add** and save). Verify only `account_info.read`, `files.metadata.read`, `files.content.read`; never add write permissions. Development-mode test accounts require owner approval. Production status requires independent Dropbox approval and is not established by CI.

## C. Fixed installed candidate — actual acceptance

Only after an exact-head Windows installed-feature build and CI, test a **separately identified installed candidate** on a disposable Windows 10/11 profile and Development-authorized Dropbox test accounts.

1. Click `Войти в Dropbox`. Observe a real **default HTTPS browser** opening `https://www.dropbox.com/oauth2/authorize`, **not** Explorer. Successful `ShellExecuteW` return alone is insufficient. No browser is FAIL.
2. Consent within the 120-second callback deadline. Expect `ACCOUNT_AUTHORIZED / VAULT_NOT_APPROVED / SYNC_DISABLED`. Timeout, wrong redirect/scopes, or denied consent are FAIL/BLOCKED, not PASS.
3. Quit **all** Akula processes, restart the same installed candidate and click `Проверить сохранённый вход`. Expect `ACCOUNT_SESSION_RESTORED / VAULT_NOT_APPROVED / SYNC_DISABLED` without a browser. This checks OS keyring refresh and current Dropbox identity, not publisher trust.
4. With the first identity configured, attempt `Войти в Dropbox` as a different test account. Expect `DROPBOX_ACCOUNT_OR_ROOT_MISMATCH / SYNC_DISABLED`; no new token or changed identity. To switch intentionally, first click `Забыть аккаунт на этом устройстве`.
5. Forget the saved account. Expect `LOCAL_CREDENTIAL_FORGOTTEN / SYNC_DISABLED`; after restart, no recovery until new consent. Forget is local, not a remote Dropbox revocation.
6. Optional independently pinned read-only Vault HEAD proof can report `PUBLISHER_NOT_APPROVED / SYNC_DISABLED`. Never infer upload, download, delete, sync or trust from OAuth consent.

## D. Exact-head promotion gate

Require Windows installed-feature `cargo check --locked`, Rust formatting, Dropbox account and secret-store tests (including rejected URLs, account mismatch, missing keyring), Main PR gate and existing installed cloud dialog UIA. Mock UIA confirms the button exists; **it does not prove the real browser or live OAuth**. Before public release, owner must verify B and C on the reviewed installed binary. No source merge automatically updates 0.3.14.

Record only Windows version, exact tested SHA/build, test step, PASS/FAIL/BLOCKED, safe public status codes, actual browser-vs-Explorer observation and post-restart keyring recovery. Never log OAuth URLs with query strings, callback data, access/refresh tokens, credential-store values, account IDs or private Vault details.
