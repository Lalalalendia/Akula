# AK-YD-LOCAL-12 — explicit OS-token inspection and local forgetting

## Scope
Small, independently accepted API extension to the Yandex Disk read-only connection lab, with no remote calls and no access to user files.

- `LocalCredentialScope::from_public_hint(subject, saved_app, compiled_app, workspace_uuid, profile)` checks exact product OAuth app identity before constructing an OS-keyring namespace. Untrusted public JSON cannot choose another app or another workspace.
- `inspect()` checks OS keyring and returns `Missing` or `PresentUnverified`; it never reports an authenticated session, tested file access, or Sync readiness.
- `forget_local()` removes only the exact OS refresh token and requires a verified absence (merged #952). No Yandex OAuth server-side revocation is attempted.
- No browser, token refresh, remote HTTP, Sync Core write, scheduler, background work or install/release change. This crate remains opt-in.
- Old `lib.rs` is 438 lines before and after, preserving the source-size ratchet.

Tests: reject different OAuth apps, malformed account and workspace scope, and no-feature OS keyring use. The OS-keystore adversarial backend tests from #952 remain the authoritative proof of deletion/readback behavior.

## Acceptance
```sh
cargo test --manifest-path modules/yandex-disk-connect-lab/Cargo.toml
cargo check --manifest-path modules/yandex-disk-connect-lab/Cargo.toml --features system-store
```
plus required Application checks and isolated Windows/Linux.

**Next:** native Settings can use this API to display an unverified local keyring presence and to trigger explicit local-only forgetting. Public account binding data comes from AK-YD-PUBLIC-11 (#953); no user-facing button is included here.

Reference: https://yandex.ru/dev/id/doc/ru/tokens/token-invalidate
