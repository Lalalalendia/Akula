# AK-P2P-DEVICE-IDENTITY-01 — real OS-backed Iroh identity

Status: opt-in Core implementation and separate diagnostic CLI; GUI unchanged. This
is a reusable key-custody seam, not a separate network stack or File Gate permit.

## Product objective

An Akula installation on Windows keeps the same 32-byte Ed25519 Iroh SecretKey
across process restart and later reuse through the existing Staya/Iroh adapter.
Enrollment must be explicit. Missing or damaged OS storage must never silently
regenerate the endpoint key or fall back to plaintext appdata.

## Implementation

- Core feature p2p-device-identity exports reusable load() and initialize().
- Windows stores a version-tagged 72-byte record in Credential Manager via
  CredReadW / CredWriteW, CRED_TYPE_GENERIC and CRED_PERSIST_LOCAL_MACHINE.
  Credential name: Lalalalendia/Akula/P2P/IrohEndpoint/v1.
- The record contains a marker, public Ed25519 key and secret seed. On load,
  exact length, marker and public-key consistency are checked. Secret bytes use
  zeroizing wrappers; the returned Windows-owned buffer is scrubbed before
  CredFree. DeviceIdentity deliberately has no Debug formatter.
- Initialization refuses existing credentials. No rotate/delete/reset API is
  exposed. Secure random or OS store failures refuse, rather than switching keys.
- with_secret_seed() can construct the existing Iroh SecretKey::from_bytes.
  There is no new network transport or second PKI.
- An optional Core CLI binary handles `--init` (explicit enrollment) and
  `--id` (read-only public key). The ordinary Native GUI has no new module,
  embedded CLI or transport. First-party pairing UI is still missing.
- Linux returns UnsupportedPlatform until an OS Secret Service backend is
  correctly integrated; no environment, file or plaintext fallback exists.

## Acceptance

1. Windows CI initializes a unique disposable Credential Manager entry,
   closes its object, reopens it from a *different operating-system process*,
   verifies the public key is identical and rejects a second enrollment.
   The disposable test credential is deleted after the test.
2. Windows optional Core CLI must compile and Core strict module Clippy and
   Rustfmt must pass. Linux must reject unsupported storage.
3. Next product gate: user-confirmed peer fingerprint and persistent pin/revoke;
   real QUIC remote_id() revalidation; Staya consent fence; private verified
   Bao stage and File Gate explicit create-new publication; two real installed
   Windows PCs rather than two temporary processes on one runner.

## Security boundaries

Credential Manager generic entries are protected by Windows per-user OS
facilities, but same-user processes can read or replace them. This is not
TPM isolation, hostile same-user resistance or trusted installer/Broker policy.
The bounded named mutex only serializes cooperating enrollments in the same
Windows logon session. Multiple sessions and hostile writers remain a threat.
EndpointId authenticates transport key ownership, not Vault membership,
authorization to GET, local file mutation, receipt or durable ACK.

Default production builds remain unchanged and opt-in P2P identity is disabled.
Linux/macOS OS stores and first-party runtime/UI integration are NOT done.
Related PRs: Akula #531 (owner roster), #562 (Iroh identity/revoke), #566 (real
Akula/Staya interop), Staya #79 (provider consent).
