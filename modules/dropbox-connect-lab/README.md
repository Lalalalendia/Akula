# Dropbox T1f: opt-in read-only QA probe

This standalone CLI is for an authorized developer test Dropbox account, **not** the normal Akula GUI or production sync. It never uploads, deletes or applies notes. It is deliberately unavailable without an independently obtained BLAKE3 digest of the expected Dropbox vault head. A successful probe returns `PUBLISHER_NOT_APPROVED`, **not** a trusted Vault or enabled sync.

## Offline preflight

Configure a distinct **Scoped access → App folder** Dropbox OAuth app with scopes `account_info.read files.metadata.read files.content.read`. Register one exact redirect such as `http://127.0.0.1:47219/dropbox/callback` in the App Console. The localhost HTTP exception is documented by Dropbox. Use **no write scope** for this test. Do not reuse Dropbox Drop/Inbox app credentials.

For an offline validation of the non-secret parameters:

```sh
cargo run --manifest-path modules/dropbox-connect-lab/Cargo.toml \
  --bin dropbox_readonly_probe -- \
  --oauth-app-key APP_KEY --registered-port 47219 \
  --account-id dbid:ACCOUNT --root-namespace-id 123456 \
  --vault-id 12345678-1234-1234-1234-123456789abc --profile-id test \
  --trusted-head-blake3 0000000000000000000000000000000000000000000000000000000000000000 \
  --dry-run
```

The all-zero digest above is a **syntax-only placeholder**, not a trusted anchor. Offline output is `CONFIG_VALID_OFFLINE`; it does not authenticate anyone or store anything.

## Real, explicit read-only test

1. Replace every placeholder with the verified test account/App Folder scope, correct registered redirect and **independently trusted exact head digest**. Do not derive trust solely from what Dropbox returns. Remove `--dry-run`.
2. The CLI binds 127.0.0.1 before opening the system browser. The OAuth URL, state, code, tokens and raw head are never printed. Do not run through shell tracing or CI with credentials.
3. Complete consent in the system browser within 120 seconds. It asks only for read scopes, verifies account/root, pinned revision and both digests, then saves refresh token to the operating-system secret store. The OS keyring must be available; there is no file fallback.
4. The only positive output is `REMOTE_PINNED_READ_OK` and `PUBLISHER_NOT_APPROVED. SYNC_DISABLED`. Neither proves an Ed25519 publisher policy, establishes freshness against a malicious Dropbox, or authorizes file writes.
5. Record only success/failure, commit SHA, OS/runtime, scoped test Vault ID and redacted metadata. Do not record app-specific credentials, authorization URL, code, state or tokens.

**Requirements:** a desktop environment with a system browser; Windows Credential Manager, macOS Keychain or Linux Secret Service; and an existing pinned test Vault head. Current tests cover Linux compile and fake-port contracts. An installed Windows/macOS/Windows 7 test and genuine Dropbox test account are separate acceptance gates.

Source: [Notion live QA checklist](https://app.notion.com/p/40232a84beec81be9e10d428547df0fb) · [Dropbox Authorization](https://docs.dropboxapi.com/riviera/docs/get-started/authorization).
