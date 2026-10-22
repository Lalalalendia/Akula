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

## NODE-E2E-02a — reuse Node's durable replica as Staya source (feature gated)

The **existing** `interop/akula-staya` crate now has the explicit opt-in `node-replica-transport-lab` feature. Linux-only integration test `tests/node_replica_real_quic.rs` builds on NODE-E2E-01a/01b from [Akula PR #607](https://github.com/Lalalalendia/Akula/pull/607) without inventing a parallel writer, queue format or network transport.

One operator-allowlisted Markdown fixture is staged by Core `SqliteDagStore`, and the original source is removed. Next, a synthetic owner-signed exact VaultId/DocumentId/RevisionId/BLAKE3/NodeId/DeviceId read grant and live test device-key signature pass `read_with_device_proof`. Wrong grant epoch and a different test device are denied **before plaintext is exported**. The test materializes the admitted bytes in an isolated disposable file because the current Staya *preview binary* accepts file paths, not NodeStore streams. It then uses the already merged real Akula OfferSession → QueueSelection adapter, launches actual Staya sender/receiver via Iroh QUIC/Bao, proves pre-arm GET denial and an unrelated Iroh peer denial, transfers exactly the approved root, verifies independent staged bytes after the source file is absent, and leaves File Gate/APPLIED untouched.

**Not production:** ephemeral owner/device keys, operator-provisioned trust and a manually chosen receiver EndpointId; **no cryptographic proof that the signed DeviceId is the same key as the live Iroh EndpointId**, and no STY-DEVICE-01a enrollment (#531), persistent challenge replay protection, TLS/QUIC broker or OS-owned epoch store. This is an end-to-end **lab transport proof**, not a licensed/paired user client or read authority. The temporary plaintext materialization must be replaced with bounded private NodeStore streaming/FS custody before shipping.

The new test is invoked only on Linux in the existing pinned-Staya workflow after the ordinary real two-process test. The ordinary Windows integration remains enabled; no production feature or UI is changed. NODE-E2E-02 / NODE-E2E-07 remain **NOT ACCEPTED** until real independent peer enrollment, current revoke witness, and an actual VPS test are proven.
