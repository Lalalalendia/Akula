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
