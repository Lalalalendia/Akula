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

## OS-backed Akula identity → Staya QUIC transport (opt-in)

The opt-in `os_peer::OsPinnedEndpoint` is a new admission bridge over the
**existing Iroh transport dependency**, not another synchronization engine.
On Windows it loads the long-lived private key from Windows Credential Manager
and checks the user-selected complete EndpointId against the authenticated
QUIC `remote_id()` and the current OS-protected peer pin. It repeats the
persistent confirmation/revocation check before each NEW logical operation.
No implicit first-run enrollment, discovery, key export, File Gate grant,
file access, durable ACK, background Sync, or installed UI is added.

For a manual first LAN handshake, on BOTH Windows hosts initialize the existing
opt-in `akula-p2p-identity` tool, print each `--id`, then enter the OTHER full
EndpointId with interactive `--peer-confirm`. From the repository root run:

```powershell
cargo run --manifest-path interop/akula-staya/Cargo.toml --features os-pinned-peer --bin akula-p2p-ping -- listen OTHER_ENDPOINT_ID
```

On the other host, take the listening host's actual LAN IPv4 and printed UDP
port (not a `0.0.0.0` wildcard) and run:

```powershell
cargo run --manifest-path interop/akula-staya/Cargo.toml --features os-pinned-peer --bin akula-p2p-ping -- connect OTHER_ENDPOINT_ID 192.168.1.10:PORT
```

This exchanges bounded `AKULA_HELLO_V1` / `AKULA_OK_V1` only. No WAN relay,
file transfer or user-facing sync is claimed. The CI extension runs real
QUIC on disposable loopback with Windows OS-held identity and a synthetic
test key, checks missing pin, authenticated ID, and durable revoke on an
existing session; Linux proves unsupported OS storage fails closed.
Human two-device pairing and Akula File Gate are still separate gates.
