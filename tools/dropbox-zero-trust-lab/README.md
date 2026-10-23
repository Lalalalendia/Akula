# AKULA-DROPBOX-ZT-P01..P03 - opt-in disposable Dropbox experiment

## Scope, NOT product functionality

This is an isolated lab harness. It does not integrate Dropbox into Akula and
is **not** a pass for any P2P or Drop release gate. All external cloud providers
are untrusted. A successful API conformance test does not prove that the cloud
provider will be honest or available in the future.

The script contains no access tokens, uses only Python's standard library,
performs no deletes, and does not autostart in CI. It writes only a single,
explicitly selected synthetic experiment per invocation, with both `--run` and
`--case P01|P02|P03` plus disposable-account safeguards. It never reads Akula Vault.

## Required isolated test account

1. Create a **new, disposable** Dropbox developer application configured as
   **App folder** (NOT Full Dropbox) on a **dedicated disposable account**.
   Do not re-use the Dropbox Sync app or any production Akula Vault credentials.
2. Grant `account_info.read`, `files.metadata.read`, `files.content.read`, and
   `files.content.write`. Configure OAuth in the standard Dropbox developer
   console; for a local one-shot lab, a short-lived test access token is enough.
3. Confirm the expected `dbid:` value independently from the test account.
4. Supply all credentials through an OS environment/secrets manager. Never paste
   them into the chat, commit them, or record presigned URLs in Notion/CI logs.
5. Reserve roughly 10 MiB of disposable space. The script intentionally leaves
   its uniquely named test files for manual inspection and cleanup.

## Execute (only on disposable account)

```sh
export AKULA_DROPBOX_TEST_TOKEN='(set securely from test account)'
export AKULA_DROPBOX_TEST_ACCOUNT_ID='dbid:(expected disposable account ID)'
export AKULA_DROPBOX_DISPOSABLE_CONFIRM='YES_DISPOSABLE_APP_FOLDER'
python3 probe.py --run --case P01
```

Running without `--run` produces `NOT RUN` and no Dropbox requests.
Running with `--run` but without `--case` returns `HOLD` before any network request.
Only **one** case is enabled for each invocation; do not run P02 (8 MiB+1
synthetic upload) unless the disposable account has adequate free quota and
its owner explicitly approved that specific experiment.

Before any live test, run the credential-free offline contract suite:

```sh
python3 -m py_compile probe.py test_probe.py
python3 -m unittest -v test_probe
```

The suite contains **11 mocked checks** for P01/P02/P03 and common fail-closed
behavior. No real Dropbox service is contacted during these tests.

P01: Mint **two distinct** one-use temporary upload links for the same random
exact path, both using `mode=add`, `autorename=false`, `strict_conflict=true`.
Upload A using link1. Try upload B using link2; verify original `file_id`,
`rev`, size, Dropbox content hash, exact revision bytes, and no auto-renamed
sibling. `PASS` only if the conflicting second write fails and A remains.

P02: Mint one new link for a random path; send exactly **8 MiB + 1 byte** to a
receiver that agreed to accept at most 8 MiB. If Dropbox accepts, report
`NO_8M_SERVER_CAP_OBSERVED`. That is a **negative security finding**: the
Dropbox API did not enforce the Akula-specific per-invitation budget; the
receiver must still reject the oversize object, but cloud quota was consumed.
If Dropbox rejects, report HOLD until the specific reason is known. Do not
mistake generic account or upload limits for per-invitation enforcement.

P03: Create one random path and upload 1024 bytes, then deliberately ignore the
application's knowledge of the POST result. Recover by read-only metadata,
`file_id/rev`, bounded `rev:` download, Dropbox hash and independent exact
byte-for-byte comparison. **No blind retry.** This is application-layer ACK
loss, not yet a real TCP-drop fault-injection test.

## Evidence and limitations

The output includes only random experiment IDs and sanitized HTTP status,
object metadata and PASS/HOLD/FAIL. It intentionally excludes access tokens,
passwords, temporary upload URLs and user filenames. Dropbox `content_hash`
is SHA-256 over per-4-MiB SHA-256 blocks, not Akula BLAKE3. Byte equality is
proven independently by readback in this lab; Akula's own cryptographic sender
signature/AEAD/BLAKE3 and durable Inbox remain **separate required gates**.

For each experiment, do not report live PASS without actually executing
this harness against the deliberately authorized disposable account. Do not
run it against a production, shared, or non-App-folder Dropbox credential.

References:
- https://docs.dropboxapi.com/dropbox-api/api-reference/user-endpoints/files/get-temporary-upload-link
- https://docs.dropboxapi.com/dropbox-api/docs/file-access
- https://docs.dropboxapi.com/dropbox-api/docs/technical-reference/content-hash
- https://docs.dropboxapi.com/dropbox-api/docs/error-handling