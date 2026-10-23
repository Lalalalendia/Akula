# AK-P2P-REAL-STAYA-01 — executable Akula-to-Staya interop

**Status: isolated opt-in integration, not a user-facing production transfer.**

This directory contains a **real compiled adapter** between Akula Core
`OfferV1 / DecisionV1 / OfferSession` and the separately maintained
`staya-transfer::queue::QueueSelection`. It intentionally does not mirror or
duplicate Staya's queue implementation. The dependency is pinned to Staya's
immutable commit `ccfe82dc6125e2810e95043a6a4ac55954b710e0`.

The CI workflow checks out this exact Staya revision, builds its existing
`staya-direct-preview` executable with its locked release compatibility
manifest, then runs an actual Iroh QUIC transfer between a sender and receiver
in **two independent operating-system processes**, using a real Akula
receiver-approved subset and a Staya queue commitment.

## What the roundtrip must prove

1. Create three **disposable synthetic files**, with UTF-8 Akula display labels.
2. Initialize a real Staya sender and receiver, and parse their actual Iroh
   EndpointIds from the receiver's public-key announcement and sender's
   serialized endpoint ticket.
3. Create a valid Akula Offer, then receive explicit consent for files #0 and
   #2. A forged commitment must fail before any receiver GET is started.
4. Materialize the exact approved roots and sizes into the **actual Staya**
   `QueueSelection` using restricted ASCII transport aliases
   (`object_00.bin`, `object_02.bin`).
5. Call the reusable `verify_staya_invitation` bridge: decode the real
   serialized Iroh endpoint ticket and compare expected endpoint, every root,
   size, order, alias, and Staya commitment against the approved queue.
   Mutated tickets and unselected root #1 fail **before GET**.
6. An outsider receiver with a different real QUIC key must be refused.
7. Send the invitation only after Akula consent. Receive actual Bao ranges,
   then independently read the private staged files, checking byte identity,
   exact BLAKE3 and queue progress. No Akula vault is opened.
8. Do not claim `File Gate`, `APPLIED` or a durable ACK. All CI steps must
   run on the exact head. Pinned Staya builds on Linux and Windows runners.

## Not a production authorization

This demo necessarily trusts a **local test operator** to supply two temporary
Akula DeviceIds and identify the initial Staya endpoint ticket. The gated
provider now denies real QUIC GET before the controller explicitly arms the
sender after Akula consent, but the test controller is trusted local input.
This **does not** establish a production entitlement against hostile local clients.

The next product step is a real Broker-enrolled
`Akula DeviceId <-> authenticated Iroh EndpointId` mapping and durable
single-root, time-bounded policy grant checked by the Staya provider before
every new GET. On-disk revocation and crash-stable replay checks remain
required. User file source descriptors require a race-free open and pinned
root, and publication requires Akula File Gate. None of these security
conditions can be inferred from this end-to-end test.

### Limitations and build rules

- This package is outside the shipping `aquilum-app` workspace; feature
  `p2p-offer-lab` stays disabled in released Akula binaries.
- No modification of the shipping app's `Cargo.lock`, UI, File Gate,
  network permissions or installed updater.
- The standalone bridge manifest currently resolves its own lockfile in
  CI. **Do not release** before pinning and reviewing this resolved lockfile.
- For a local run, build the exact Staya pinned
  `interop/release/Cargo.toml` binary and set `STAYA_PREVIEW_BIN`.
  The workflow is the primary supported Linux/Windows reproduction.

## Provider-side Akula consent gate (STY-G1n)

The interop test now runs real Staya `serve-batch-gated`, not an already
permitted `serve-batch`. The original receiver EndpointId must first fail a
real QUIC connection/GET. The test controller validates Akula OfferSession
consent and the actual Staya ticket/selected-root commitment, then submits a
sender-local `AUTHORIZE <queue-commitment> <receiver-EndpointId>` command.
Only after Staya confirms `PROVIDER_ARMED` may the same receiver complete
Bao staging. An unrelated real peer is denied **after** arming. Provider
permission expires after 180 seconds and closes on sender EOF.

This is a **synthetic two-process integration test**, with a trusted local
controller and ephemeral device identities. It is not a real Akula installed
UI, trusted device enrollment, remote-signed user decision, authenticated
host IPC, File Gate publish permission, or durable application receipt.
Do not enable shipping P2P from this result.

## Receiver create-new inbox (Akula P2P INBOX-01)

After an actual Staya QUIC/Bao transfer produces a private staged file,
`aquilum_core::p2p_inbox_lab::save_verified_stage_as_new` accepts its
**already opened File handle** plus the root/length from independently
approved Akula OfferSession. It copies through a 64 KiB buffer into a fresh
sibling candidate, cold checks source size and BLAKE3, flushes the candidate
and uses a **same-directory no-clobber hard link** to publish the
user-selected NEW destination. Existing files/symlinks, changed stage sizes,
wrong roots, and unsupported hard-link filesystems fail closed; no rename
fallback. A wrong digest never makes partial destination bytes visible.

This is now exercised with the actual Staya staged bytes in the Windows/Linux
two-process E2E and also through portable Core unit tests. The verified stage
remains separate from the user destination; a second attempted save to that
destination is rejected without mutation.

**Important limits:** this is a Core opt-in feature / test path, not yet a
button in installed Akula; neither the receiver's selected destination nor
the sender's QUIC identity is currently driven by the real File transfers
window. The API cannot prove the provenance of caller-supplied expected
BLAKE3 roots; the caller MUST use an authenticated accepted OfferSession.
It does not protect all ancestor path races or guarantee directory metadata
durability after power loss. A successful copy is not File Gate APPLIED,
Vault synchronization, or a durable revision ACK.

## AK-P2P-LIVE-ID-01 — live Iroh key and fresh protected peer pin (opt-in)

The standalone `p2p-live-identity` feature derives the actual Iroh
`SecretKey` from Akula's existing OS-backed `DeviceIdentity`, checking
the resulting public ID. It exposes `require_pinned_live_peer(connection,
selected_id)`: this reads cryptographically authenticated Iroh
`Connection::remote_id()` and delegates to Core's OS-persisted
`peer_pin::require_pinned_remote` on **every new operation**. No claimed
network ID or invitation-derived key is accepted as proof.

Linux/Windows tests establish a genuine loopback QUIC connection and prove
wrong selected IDs and a fresh unconfirmed peer fail closed. Test code does
not initialize, replace, delete, or leak any user's canonical credentials.

**Not installed:** neither native File transfers nor the Staya provider calls
this seam yet. No DeviceId/Vault/object authorization, sender consent, local
IPC control, user-file GET, File Gate apply, durable ACK or two-PC acceptance.
The existing receiver create-new and OS identity code stay opt-in. This work
cannot enable network file access.

## NODE-E2E-02a — offline verified Node replica via Staya (lab)

The opt-in `node-replica-transport-lab` test persists one allowlisted Markdown fixture through the existing Sync DAG. After its original source is removed, a signed exact read grant and synthetic device proof allow a one-time disposable export. Actual Staya Iroh QUIC/Bao transfers verified bytes to a separate process. The provider refuses pre-arm GET and an unlisted Iroh peer.

This does **not** enroll a real user device, grant File Gate rights, provide a server Node daemon or remove the temporary plaintext path. Existing `p2p-live-identity` and `p2p-inbox-lab` retain their separate validation gates. Product NODE-E2E-02/07 remain unaccepted; see [AKULA-NODE-E2E-01A](../../docs/platforms/AKULA-NODE-E2E-01A.md).
