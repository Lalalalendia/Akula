# Akula: Yandex Object Storage onboarding (YC-S3-T01 / stage 1)

The existing native **Settings → Cloud & recovery** page now includes a
beginner-readable Yandex setup surface. It is deliberately a **draft**, not a
connected cloud session. Do not announce a shipping S3 connection yet.

## What the user can do

1. Open the official Yandex Cloud console and instructions for a service account.
2. Open the official instructions for a static S3 access key.
3. Open the instructions for creating a private bucket. Cloud operations may be billed.
4. Enter the **non-secret bucket name** and **name of an external AWS profile**
   (never the profile contents); press Enter to save each in app settings.
5. See the pinned Yandex S3 endpoint `https://storage.yandexcloud.net`
   and region `ru-central1`, without guessing provider-specific values.

The screen includes read-only connection status **Not connected** and a disabled
`Check connection` control. No S3 worker starts, no bucket is created, no
trust authority is changed, and no original local Markdown is written.

The UI has no `Access Key ID` or `Secret Access Key` input. **Do not enter a
key into the bucket, profile name, search or other plaintext fields.** The
native `TextEditor` settings widget is not a secure input, and the app config
is ordinary JSON. The only persisted values are public bucket and profile
names. The profile label is a draft; the SDK does not consume it yet.
The current default native build does not activate the core `sync-s3`
feature. A prepared draft must never be misrepresented as a successful
connection to a Yandex bucket.

## Next gates, in order

**Stage 2 — Secure key entry on desktop.** Build masked secret fields with
clear-on-close, no clipboard prefill, no debug/display serialization and safe
error reporting. Store the key pair in the platform credential vault
(Windows Credential Manager / macOS Keychain / Linux Secret Service or an
explicit equivalent); save only a scoped opaque reference in app-data.
Implement replace/revoke/clear and sensitive-memory handling. Do not use
app-config, Sync journal, vault, Notion, GitHub or logs for key storage.
Respect the macOS release hold until reopened.

**Stage 3 — Real read-only connection probe.** Explicit user action only,
limited `HEAD Bucket` or bounded `LIST` to a configured prefix, with a
fixed allowlisted `https://storage.yandexcloud.net` destination and
`ru-central1`. Use the existing official AWS SDK S3 transport and SigV4;
do not interpret Yandex IAM bearer tokens as S3 credential pairs. Display
distinct missing/invalid credentials, access denied, unknown bucket, TLS,
timeout, wrong region, insufficient permissions. Never log SDK request
authorization headers, key identifiers or plaintext keys. No object writes
or automatic retries on auth denial.

**Stage 4 — Yandex-specific scope and trust.** A successful S3 response does
not prove the vault owner or manifest publisher. Require explicit verified
remote Vault ID, expected publisher key and custody review before binding
an existing local knowledge base. Unknown/empty remote roots cannot silently
bootstrap protected writers. Run real Yandex tests on a private testing
bucket with explicit user approval, *then* conditional CAS, multipart,
concurrent clients and safety gates per SYNC-05. Never enable destructive
Sync until all release gates pass.

## Acceptance for this PR

- RU and EN have all labels and actionable Yandex documentation links.
- Bucket/profile names survive restart in ordinary settings; no secret
  fields or AWS credentials enter those settings.
- Untrusted URLs are never clickable through generated form values.
- Input validation denies URLs, Unicode pasted secrets, path traversal,
  shell metacharacters and oversized names.
- The S3 test control cannot be invoked and the status never says connected.
- The previous native recovery-only/forensic review controls stay intact
  and maintain their existing fail-closed semantics.
- CI proves the schema wiring, serialization and tests on exact commit.
- The screen is an **onboarding prerequisite**, not the requested final
  paste-and-connect wizard until Stage 2–4 above are implemented.

References:
- [Project Yandex S3 spec](https://app.notion.com/p/3fe32a84beec8118a2b4e31f64434602)
- [SYNC-05](https://app.notion.com/p/3fc32a84beec818799b3c7acd3804fd3)
- [Yandex Object Storage quickstart](https://yandex.cloud/ru/docs/storage/quickstart/quickstart-aws-cli)
