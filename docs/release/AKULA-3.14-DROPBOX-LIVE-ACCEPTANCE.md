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


## F. Automated acceptance evidence — 2026-10-25

**Accepted synthetic checks; no real Dropbox consent yet.**

- [Windows Dropbox installed-feature and account tests #40185782800](https://github.com/Lalalalendia/Akula/actions/runs/40185782800) — PASS at `626928c176cf960b35e9314cea96db32e2b5d4b9`; Windows `cargo check --locked`, native Dropbox state/configuration tests, isolated onboarding tests.
- [Dropbox T0 / PKCE / OS credential contract #40185782816](https://github.com/Lalalalendia/Akula/actions/runs/40185782816) — PASS at the same SHA.
- [Windows NSIS installed candidate #40185905872](https://github.com/Lalalalendia/Akula/actions/runs/40185905872) — PASS at `9454668a1c86b6b1468ae991be02b13ce10115bb`. Built `Akula_0.3.14_x64-setup.exe`, installed and exercised editor UI, persisted edits across a restart, verified OS credentials and uninstalled. This did **not** click Dropbox OAuth or prove real browser navigation.
- Installer `SHA-256: 28cce3bb82c76ec17b44a46f17b6b98f9601f7e0cec470a251cba12ee5b3b5ba`; `19,597,208` bytes; manifest `actual_checkout_sha=9454668a1c86b6b1468ae991be02b13ce10115bb`. CI artifact: `akula-windows-beta-candidate-40185905872-1`.
- This candidate is **UNSIGNED**, carries the unchanged source package version `0.3.14`, and must only be tried on an isolated test account / VM. It is not the published signed v0.3.14 and is not a secure auto-update. No release signing or production Dropbox approval has occurred.
- Temporary push triggers used to obtain the three checks were removed in the subsequent branch-only commit `87621017b67c6b32f5f5e393cf1d422acdfef242`. Final source code for the launcher and account binding is unchanged; workflow definitions again match `main`. Required PR `Main PR gate` has **not** run.

**Remaining HOLD:** Perform C1–C5 with a real Development-authorized Dropbox account on this exact test candidate. Record browser-vs-Explorer, callback acceptance, cold-start keyring resume, fail-closed wrong-account handling and local Forget (without any secret in logs). Then pass the protected PR gate and independent signed release acceptance. Until then, do not describe Dropbox login or public Dropbox availability as fully accepted.

## G. No-account Windows browser UIA (new gate)

An additional UIA probe was added in `scripts/windows-dropbox-browser-uia.ps1` to close the gap between a successful `ShellExecuteW` return and a **real visible browser window**. It runs the installed-feature native executable on an ephemeral GitHub-hosted Windows runner, opens Dropbox settings and invokes the real account login button.

The probe snapshots visible top-level Edge/Chrome/Firefox/etc. browser HWNDs *before* the click and requires a **new browser window** afterward. It deliberately does not inspect URLs, process command lines, account identities or OAuth tokens; it never grants Dropbox consent. The existing code-level URL allowlist is tested separately.

- [Browser UIA source commit](https://github.com/Lalalalendia/Akula/commit/d90b7bccc139afb660fd1dcb30f416447a329929)
- [Existing Windows Dropbox workflow integration](https://github.com/Lalalalendia/Akula/commit/1627f3719d8a3d1ca0a40ff634e621c8fd213320)
- [First Windows browser UIA CI #40195949867](https://github.com/Lalalalendia/Akula/actions/runs/40195949867) — **FAIL: TEST HARNESS PATH**, not a browser defect. Native `cargo check`, 16 native tests, 5 config tests, 31 isolated onboarding tests and `cargo build` passed; the UIA script aborted *before clicking* because it expected `.artifacts/cargo-native/debug` while this workflow compiles from the repository root. No browser verdict was obtained.
- [Resolved-path test fix](https://github.com/Lalalendia/Akula/commit/a4e0c56a287e657986abbeb088b3eaa9bef5e429) and [workflow fix](https://github.com/Lalalendia/Akula/commit/4a81d22167c15267c6a3ad0974f407be96d04633) use `cargo metadata` and check that the executable is inside one of two approved Akula build output directories.
- [Re-run Windows browser UIA CI #40199126430](https://github.com/Lalalendia/Akula/actions/runs/40199126430) — **PASS** at `4a81d22167c15267c6a3ad0974f407be96d04633`. Windows log at `2026-10-25T13:59:26Z`: `AKULA_DROPBOX_SYSTEM_BROWSER_UIA_PASS browser_window=1 explorer_only=0 consent=0 tokens=0 sync_disabled=1`; UIA observed a new visible Microsoft Edge window after clicking the real Akula Dropbox login button. This is a **direct cargo-compiled installed-feature GUI**, NOT yet the separately packaged NSIS-installed executable.

The temporary branch-only `push` subscription in `dropbox-native-313.yml` was removed in commit `5c2cbec4e01ed8bb3e8f64c2873b1c8225ac60b2`, retaining the probe in the existing workflow's `pull_request` and manual paths. This probe does not establish real Dropbox account consent or signed/installed production release approval.

## H. NSIS-installed Dropbox browser verification — independent stronger gate

The direct native (debug) UIA acceptance above is necessary but not a substitute for verifying the **same installed EXE as the actual NSIS candidate**.

- [NSIS-specific UIA preflight](https://github.com/Lalalendia/Akula/commit/df0a0926be7331f9862241502604050a67e660ec) requires an ephemeral GitHub-hosted Windows runner, the exact `%LOCALAPPDATA%/Akula/akula-app.exe` path, Akula uninstall registration and SHA-256 equality with the compiled release `akula-app.exe`.
- [Windows beta acceptance integration](https://github.com/Lalalendia/Akula/commit/b6b8ad60b75540f5f74151e5853cb7f940e9904a) runs that same no-credential button-click/browser-window witness **after** edit/save/restart UIA and before uninstall, without granting consent or logging URLs/tokens.
- [NSIS candidate workflow #40201337194](https://github.com/Lalalalendia/Akula/actions/runs/40201337194) — **PASS** at exact tested checkout `147a28c5efe836f82e97e2aedb11691625d6f7bf` (completed 2026-10-25). Logs: `AKULA_DROPBOX_SYSTEM_BROWSER_UIA_PASS browser_window=1 explorer_only=0 consent=0 tokens=0 sync_disabled=1`, `AKULA_WINDOWS_BETA_REOPEN_GUI_READBACK_PASS`, `AKULA_WINDOWS_BETA_GUI_EDIT_PASS` and `AKULA_WINDOWS_BETA_PROVENANCE_PASS`. The CI manifest reports installer SHA-256 `628725545330091fa955318f06b1129130cc0b00c2d8408e0fbcbcc094994ff6`. Artifact: `akula-windows-beta-candidate-40201337194-1` (ID `12602851688`). The earlier v0.3.14 test candidate in section F is a different binary.
- The temporary branch-only `push` trigger in `.github/workflows/windows-beta.yml` was removed after this passing run by [`deae9a5d`](https://github.com/Lalalalendia/Akula/commit/deae9a5d1bbb82a98a65e3c141f0db8e41c621d0). The original `workflow_call` and `workflow_dispatch` events remain; `scripts/test-windows-beta.ps1` retains the actual installed Dropbox browser test.
- This checks an **unsigned** CI-only NSIS candidate; even a passing run does not prove real Dropbox consent, account/session persistence in Windows Credential Manager or production OAuth app approval. No public v0.3.14 artifacts are changed.

## I. Integration decision following browser acceptance

**Native and NSIS-installed Windows browser UIA: PASS. Live Dropbox account and public release: HOLD.**

The Windows launcher correction and scope-bound re-consent are on the same development branch. The actual NSIS-installed candidate opened a new system browser window in ephemeral GitHub-hosted Windows, without submitting credentials or granting consent. This establishes the original *File Explorer instead of browser* defect is fixed for that candidate, not for the already published signed v0.3.14.

The required main-targeted PR `Main PR gate` is not yet passed. Preserve branch isolation and perform security review, exact-head CI and dependency checks without exceeding the repository's open-PR policy or pushing directly to `main`.

Production release is separately blocked on owner verification of Dropbox app callback/scopes, Development-authorized consent in an installed Windows candidate, cold restart/refresh from Windows Credential Manager, and rejection of a different account/root without token replacement. A signed public release needs its own provenance and acceptance evidence; none is claimed here.
