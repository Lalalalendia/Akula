# YC-S3-T01c-WIN-BRIDGE-0 — inert single-flight GUI probe session

This is a **prerequisite, not a connected Windows UI**. It introduces a standalone, std-only asynchronous state-machine adapter in native code. It imports the existing **public non-secret** bucket validator from Core, but no credentials or S3 client. No Windows Credential Manager read, network, upload, Sync worker, File Gate or trusted Vault bootstrap is enabled by this PR.

## Problem

A user clicks **Check connection** and then changes the bucket, replaces/deletes the keys or closes Settings. An already running request may subsequently return success. Blindly displaying that late response can misrepresent the new credential/bucket as verified.

## Contract

- A request starts **only** when a future explicit button handler invokes `ProbeSession::start` with a boolean result of a separate protected-store presence check. A boolean is not an authorization proof.
- A request carries a monotonic ticket consisting of bucket, epoch and sequence number. The session never owns key strings, request bodies, credentials, SDK client or remote data.
- Exactly one worker slot per native session. Changing bucket, keys or closing Settings invalidates the visible result but **does not free the slot** until the previous worker completes. This prevents overlapping old/new access checks.
- A worker uses `std::thread::Builder`; the actual operation is injected as a background-only closure and returns a fixed `ProbeOutcome` enum with no secret fields. The core network adapter remains separate.
- The worker communicates through mpsc and the existing-app-compatible wake callback. The GUI thread polls for completions and accepts a result only if the returned token is still the active one, with the current bucket and epoch.
- The only success status is `ReadOnlyListVerified`. It never means remote Vault owner, upload permissions, usable Sync writer, CAS capability or safe local write permission. Closing Settings resets visible success.
- The window should not show cached success after restart. All statuses are transient/in-memory. Full integration still must invalidate on **every** credential Save/Replace/Delete operation and account/bucket change. A direct OS change outside the app cannot be detected reliably without an additional credential identity check.
- The future background operation must retrieve the credential from the approved WinCred API *inside its worker closure*, and may then call Core `probe_yandex_s3_read_only` through a bounded runtime. It must not return or log the credential values.
- This is per-window/session single-flight, **not** a global OS-wide lock. Concurrent windows/processes require separate authorization and resource controls before enabling automatic polling or Sync.

## Deterministic tests

Required Windows beta cargo tests validate: blank/bad bucket; no keys; success status never implies Sync; closing invalidates; key rotation invalidates an in-flight success; no second probe before previous worker ends; a changed bucket rejects the old result; successful fresh retry; failure status; duplicate polling is idempotent.

## Release HOLD

- [PR #388](https://github.com/Lalalalendia/Akula/pull/388): Windows key storage, **physical masked CredUI QA**, exact-head CI, merge.
- [PR #408](https://github.com/Lalalalendia/Akula/pull/408): production Core Yandex read-only SigV4 probe, exact-head CI, merge.
- A separate integrated bridge (with SDK compile feature isolated from default native binaries, cleanup and localized UX) must be tested with fake workers and credentials before enabling Check connection. Do not enable automatic Sync or writes.
- Real live Yandex probe on a user-authorized **disposable private** bucket with bucket-scoped read-only permissions is an independent acceptance gate.

This PR is intentionally Draft until its own Windows beta CI and test cases pass. No actual Yandex credentials were used.
