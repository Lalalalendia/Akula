# AK-P2P-REAL-STAYA-01 — executable Akula-to-Staya interop

**Status: isolated opt-in integration, not a user-facing production transfer.**

This directory contains a **real compiled adapter** between Akula Core
`OfferV1 / DecisionV1 / OfferSession` and the separately maintained
`staya-transfer::queue::QueueSelection`. It intentionally does not mirror or
duplicate Staya's queue implementation. The dependency is pinned to Staya's
immutable commit `fadb38a792126f6098df086fc56ce0dcae3ca3f1`.

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
Akula DeviceIds and identify the initial Staya endpoint ticket. Staya's
existing preview provider starts listening before it learns of Akula consent;
the test controller withholds the invitation until consent but this does
**not** prove a production enforcement fence against all possible clients.

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
