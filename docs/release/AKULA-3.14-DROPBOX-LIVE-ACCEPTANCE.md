# Akula 0.3.14 — Dropbox live OAuth acceptance (owner only)

## Release boundary

Windows signed [v0.3.14](https://github.com/Lalalalendia/Akula/releases/tag/v0.3.14)
includes the native Dropbox account-only flow and read-only Vault proof. It does
**not** include Dropbox write/delete, background sync, trusted publisher policy
or a publicly approved OAuth app. The signed installer, SHA-256 and source are
already fixed; this document does not modify the published release.

## External prerequisites: Dropbox developer owns these settings

In the [Dropbox App Console](https://www.dropbox.com/developers/apps), select
Akula's app `gzrirdrrx4tarly`, Scoped access **App folder** (folder `Akula`).
Verify that the **saved** OAuth Redirect URIs list contains exactly
`http://127.0.0.1:47219/dropbox/callback`; typing the URI without clicking
**Add** does not register it. Enable and save only `account_info.read`,
`files.metadata.read`, `files.content.read`; do not request write scopes.
This is a **Development** app until Dropbox independently approves Production
and cannot be advertised as generally available. Never send its secret key
or real account tokens in a PR, issue, chat, screenshot or CI log.

## Live acceptance (not possible in repository CI)

Use a disposable Windows 10/11 test user + browser and the **ordinary signed
Akula v0.3.14**, with one installed Akula, no user Vault or real notes.
Enable Windows Credential Manager. Use a Dropbox account that the developer
has authorized for Development testing. Do **not** run this against valuable
files or impersonate a production user.

1. Open Akula → Settings → Cloud → Dropbox. Confirm the modal opens and starts
   `DISCONNECTED / SYNC_DISABLED`.
2. Click `Войти в Dropbox`. Complete consent in the system browser within
   120 seconds. Confirm a status `ACCOUNT_AUTHORIZED / VAULT_NOT_APPROVED /
   SYNC_DISABLED`. No VaultId or independent HEAD pin is required for login.
3. Close all Akula processes and reopen the **same installed** application.
   Click `Проверить сохранённый вход`. A successful refresh must show
   `ACCOUNT_SESSION_RESTORED / VAULT_NOT_APPROVED / SYNC_DISABLED`, with no
   browser. This proves read-back through the OS secret store plus live Dropbox
   account/root verification; it does **not** prove publisher trust.
4. Re-enter `Войти в Dropbox` using a **different** Dropbox account while
   the first identity is still configured. The next code patch must return
   `DROPBOX_ACCOUNT_OR_ROOT_MISMATCH / SYNC_DISABLED` and must not replace
   the account or store a second refresh token. To switch accounts, use
   `Забыть аккаунт на этом устройстве` **first**. Record behavior without
   recording IDs or secrets.
5. Click `Забыть аккаунт на этом устройстве`. Verify
   `LOCAL_CREDENTIAL_FORGOTTEN / SYNC_DISABLED`, then restart; recovery
   must not succeed without a fresh login. Local forget is **not** remote
   revocation. To revoke on Dropbox, use Connected apps.
6. Optionally validate a separately trusted Vault using real independent
   VaultId/profile and out-of-band BLAKE3 HEAD. A successful read-only proof
   remains `PUBLISHER_NOT_APPROVED / SYNC_DISABLED`; never infer a file
   download, upload, delete or sync guarantee from account login.

## Evidence and release gate

Record only: Windows version, Akula release/commit, outcome of each step,
safe public status codes, and whether OS keyring was used after restart.
Do not collect OAuth callback URLs, login codes, access/refresh tokens,
secret-store values or account identifiers. **No authentic Dropbox consent,
scope verification, keyring restore, or Production approval has been completed
merely by merging this change or passing synthetic CI tests.**

The account-switch binding fix in the following development PR is a **future
release** change: it does not retroactively modify the published v0.3.14 bytes.
If tests use installed v0.3.14 before that fix ships, do not expect step 4's
new fail-closed behavior. Record that limitation explicitly.

Dropbox references:
- https://www.dropbox.com/developers/reference/getting-started
- https://www.dropbox.com/developers/reference/developer-guide
