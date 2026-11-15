# AK-YD-PUBLIC-11 — account-to-workspace binding (no tokens)

## Why
After #951 the OS keyring can retain an OAuth refresh token, but a restart loses the exact Yandex subject ID required to find that entry. Do **not** assume an OS keyring entry proves live authorization. A bounded, non-secret public hint is necessary for explicit user-driven inspection and deletion.

## Behavior
- New inert `AppConfig.yandex_disk_public` field, default empty for old configs; no change to existing S3/Dropbox configuration and no user-vault files.
- Per-entry **public** Yandex subject ID, OAuth client ID, local UiState workspace UUID, profile. No refresh/access tokens, PKCE state, auth code, client secret or cryptographic Sync root.
- One entry per (local workspace UUID, profile), max 32. A different account or OAuth app for that same scope is an explicit conflict; never silently replace an account and orphan its OS keyring record.
- `unverified_for(workspace, compiled_app, profile)` only yields a syntactically checked, untrusted pointer. The native host MUST check the current product-registered OAuth app ID and OS keyring separately, and still report `SYNC_DISABLED`.
- `forget_exact` is only for use *after* a successful checked deletion from the OS credential store. It never touches another account/workspace/profile.
- Reject malformed IDs, duplicate workspace profiles, over-size lists and suspicious JSON. Unknown credential-like JSON keys are discarded on serialization. Saved settings are never authorization proof.
- Core file size is unchanged at 551 lines under Akula's no-growth rule; real logic/tests in dedicated new modules.

## Acceptance
Required `Application checks`, including Core tests, policy, and shared-manifest security lanes; no bypass.

Next milestone: opt-in native UI should store the public hint only after the explicit reviewed account and OS credential have passed the security contract, then show `PresentUnverified` on a manual status check and offer *Forget on this device*. If persisting the public hint fails, run an exact OS credential cleanup and do not claim connected. Refresh of a Yandex OAuth token is NOT implemented: official documentation for refresh requires client authentication using a `client_secret`; never bake a privileged secret into a distributed desktop binary. Remote revocation is a separate operation via Yandex Data Access settings.

Official docs:
- https://www.yandex.ru/dev/id/doc/ru/tokens/refresh-client
- https://yandex.ru/dev/id/doc/ru/tokens/token-invalidate
