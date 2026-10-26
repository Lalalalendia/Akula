# AK-P2P-LOCAL-ID-UI-01 — local device key in existing File transfers window

Status: opt-in, stacked on AK-P2P-DEVICE-IDENTITY-01 / PR #572. No shipping
P2P file transfer is enabled. The only user-visible addition is an explicit
first-party setup/read/copy control for this device's real public Iroh key.

## Why this step

The Windows Credential Manager-backed 32-byte seed in Core already provides a
stable Ed25519 public EndpointId, but PR #572 exposed it only through a
developer diagnostic binary. File transfers had no user action for pairing.
Do not invent a new network service, device registry or another identity.

## First-party behavior

- Compile with `aquilum-native --features p2p-device-ui` (ordinary installed
  releases still exclude this feature).
- Open existing File transfers. The optional local-identity card starts as
  **Unchecked** and does not access OS credentials on startup.
- **Read this device ID** asynchronously calls Core `load()` and reports:
  absent credential, real 64-hex public EndpointId, or a protected-store error.
  Missing/corrupt credentials never silently create or rotate a private key.
- Only after a confirmed **Missing** state can a user press **Create device
  identity**. The background worker invokes the Core `initialize()` which
  refuses replacement and uses Windows Credential Manager. If another app
  enrolled concurrently, re-read the existing identity rather than rotating.
- **Copy public ID** puts only the 64-hex Ed25519 public key on the clipboard.
  No `with_secret_seed`, credential blob, private seed, Vault/File Gate grant
  or QUIC connection is reachable from these controls.
- The existing File transfers draft picker and synthetic test-only recipient
  list remain unchanged. No device is marked trusted by this local card.
- A generation fence ignores old asynchronous responses. The event loop is
  explicitly woken on worker completion; no OS calls run in a UI press event.
- On platforms without a supported protected store, Core refuses. The UI
  shows the error and never switches to plaintext fallback.

## Validation

The new Windows-only native workflow compiles the **real** native File
transfers UI with `p2p-device-ui`, formats the new modules, and runs targeted
UI plus state-machine tests. Existing PR #572 independently verifies real
Credential Manager persistence across two separate Windows processes.

## Product hold

This is device-local bootstrap, not the sender-recipient pairing handshake.
The copied EndpointId MUST still be compared through an independently verified
channel and associated with a user-accepted peer. Receiving another person's
public key alone is not approval to transfer anything.

Remaining product chain:

1. Trusted manual peer confirmation and authentic QUIC `remote_id` binding.
2. Owner-verified exact OfferSession / one-time provider-side Staya GET grant.
3. Real sender and receiver runtime connected to this same File transfers UI.
4. Verified Bao/BLAKE3 private inbox, user Save as New, no overwrite.
5. Installed Windows two-computer acceptance, then release gates.

Related: Akula PRs #513, #528, #566, #571, #572; Staya #79.
