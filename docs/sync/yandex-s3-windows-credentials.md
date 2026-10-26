# YC-S3-T01b — Windows credential entry (stacked on #382)

Scope: native Windows UI; still **NO S3 request / remote trust / file mutation**.

## User experience

1. In Akula Settings → Cloud & recovery, read the Yandex setup instructions and enter only the private **bucket name**, then press Enter.
2. Click **Add / replace keys**. The **native Windows Credentials UI** opens. In that dialog only, enter:
   - **Username** = Yandex static **Access Key ID**.
   - **Password** = corresponding **Secret Access Key**.
3. Akula uses Windows CredUI with **DO_NOT_PERSIST** (the prompt itself does not persist a credential). Only upon confirming valid nonempty key formats does Akula use `CredWriteW` to store a **generic credential** in **Windows Credential Manager**, scoped to the current Windows logon and target `Akula/YandexObjectStorage/v1/<bucket>`.
4. Settings shows **Keys saved in Windows · access not verified** and keeps **Check connection** disabled. User can replace saved credentials with the same button, or explicitly **Remove Windows keys** after Yes/No confirmation.
5. Switching bucket names never silently reuses keys for another bucket. On Linux and macOS, key-entry actions remain disabled until separate platform implementations exist (macOS release hold still applies).

## Safety boundary

- No Access Key ID or secret value is saved in ordinary Akula settings JSON, notes, Sync SQLite journals, logs, request URLs, telemetry, or clipboard by this feature.
- Credential fields are owned by Windows's masked modal, not plaintext Akula `TextEditor`. The Windows result buffers and temporary UTF-8 buffers are zeroized after success/cancel/failure via `zeroize::Zeroizing`.
- `CredReadW` is used solely to test credential **presence**, immediately freed via `CredFree`. It does not return keys to application panels. There is no S3 connection instantiated by this PR.
- Credentials are stored using Windows **generic credentials** with local-machine persistence, protected by the Windows user logon. This is not protection from malicious software running as the same user or an administrator. User can revoke keys in Yandex Cloud.
- Microsoft Windows's built-in CredUI generic text field is captioned *Username*, not *Access Key ID*. The localized on-screen instructions explicitly map username/password to Yandex fields. This is temporary for desktop-first MVP; a custom masked in-app control must meet equivalent controls before adoption.
- On Windows, `CredReadW` presence is **not authorization**. A saved credential may be revoked, belong to an unrelated service account, or have insufficient permissions. No automatic retries, PUT, CAS, remote trust binding or Sync workers.
- No user secrets or cloud resources are needed for CI. Never paste actual keys into GitHub, Notion, issue comments, logs or chat.

## Acceptance (this PR)

- Exact-head Windows candidate build compiles `Win32_Security_Credentials`, validates `CredUIPromptForCredentialsW` and `CredWriteW` symbols, and passes native settings UI and schema tests.
- Linux native checks compile platform-safe fallback without loading Win32 code.
- Unit tests prove bucket-specific credential target and strict invalid bucket rejection, and Windows only enables credential action for a syntactically valid saved bucket.
- Existing read-only recovery dashboard and button permissions remain unchanged. The disabled S3 check control cannot be invoked.
- A human Windows A/B still must exercise **Cancel**, invalid key rejection, save / replace / restart / delete for two buckets using *test credentials* before marking the UX physically accepted. This PR should remain Draft until that evidence is available.
- Next: user-triggered **bounded read-only S3 HEAD/LIST** with typed errors, TLS/endpoint constraints, no secret logs, and separate Yandex trust bootstrapping. No connection or synchronization is claimed here.

Microsoft API references:
- https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-creduipromptforcredentialsw
- https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew
- https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credreadw

## YC-S3-T01b-READ — Windows-only in-memory credential checkout

An internal `with_stored_keys(bucket, callback)` helper can now retrieve a **single bucket-bound** generic credential through native `CredReadW`. The callback receives borrowed key strings, **not** a serialized config object. This is an enabling primitive for a later background read-only probe; no network call, SDK configuration, GUI action or Sync is enabled here.

- Input reuses the strict bucket validation and versioned `Akula/YandexObjectStorage/v1/<bucket>` key; **empty drafts are invalid**.
- The API distinguishes absent keys from unreadable/unavailable OS store and malformed stored credentials. The latter are never handed to a callback.
- Decoding checks length, ASCII formatting, NUL termination and limits of the persisted UTF-8 secret. Temporary `Zeroizing` allocations are wiped on ordinary return and unwinding. The credential's Windows-allocated copy is always released with a RAII `CredFree` guard, and is released **before** the callback runs.
- This does not protect from process-local malware, OS swap/crash snapshots or unspecified copies in AWS SDK memory. Windows Credential Manager deletion does not revoke the Yandex static key server-side.
- The native Windows test now proves **store → read exact synthetic test value → replace → read replaced value → reject malformed stored bytes → delete → missing**, plus isolation from a different bucket. The Windows QA now launches the exact compiled Rust test harness, verifies test discovery, and attempts an interactive system-modal Cancel through UI Automation. Password masking is independently verified either by UIA `IsPassword` or by native Win32 `ES_PASSWORD` on an `Edit`/RichEdit child of a window belonging to the exact test PID; neither method reads field text. The CI remains **unaccepted** until the actual Windows runner demonstrates the modal mask, Cancel and no-persistence assertions. This does not cover the separate human Save/Replace/Delete UX acceptance.
- Security boundary: the caller is responsible for scheduling the callback only on a background worker after explicit user action, preventing copied secrets in its return value, and logging only sanitized error categories. The callback is deliberately **not wired** to any production action until Core #408 is independently accepted.


### Windows UIA acceptance triage, 21 October 2026

The completed [Windows CI #39594264042](https://github.com/Lalalalendia/Akula/actions/runs/39594264042) found the compiled Rust test executable and the actual masked CredUI field, then **FAILED** with `Genuine Windows CredUI Cancel button not found`. This confirms that locating password-masked input no longer blocks the test; it does **not** establish Cancel, no-persistence or production readiness.

The next test build uses the verified test-PID window with masked field, enumerates its native child Win32 `Button` HWNDs, and looks for standard `IDCANCEL=2` without trusting localized captions. Crucially it still requires **UI Automation InvokePattern** on that exact native button HWND to perform Cancel. It never substitutes keyboard Escape, BM_CLICK, or blind SendMessage to mark an inaccessible dialog PASS. The subprocess must then exit with exactly one Rust test passed and the dummy WinCred target absent. [Exact-head run #39602360683](https://github.com/Lalalalendia/Akula/actions/runs/39602360683) is pending. If Win32 provides no invokable accessible Cancel element, physical Windows QA or a different test host is required; do not weaken the security gate.
