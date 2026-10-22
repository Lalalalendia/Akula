# NODE-LIVE-ID-01 — established Iroh identity + signed roster + exact read

**Status:** separate feature-gated implementation Draft; not a production Node, not an installed Akula test. This integration is intentionally stacked on [#607](https://github.com/Lalalalendia/Akula/pull/607) and reuses [#531](https://github.com/Lalalalendia/Akula/pull/531) after both have independent approval. Canonical design: [AKULA-NODE-ARCH-01](https://app.notion.com/p/40132a84beec8118b1a2fffc90fb808c).

## Security-critical path

The adapter `interop/akula-staya/src/node_live_read.rs` **accepts the actual Iroh `endpoint::Connection`** and obtains the peer from `connection.remote_id()` after its authenticated QUIC handshake, not from a ticket, chosen DeviceId or wire payload. It verifies a separately pinned owner-signed roster via existing `inspect_signed_roster` and requires an *exact* externally witnessed epoch/digest (not merely a rollback floor). Only one `RecordedOnly` row may match, and for Node v1 both the roster's `device_public_key` and `endpoint_public_key` must equal this actual live Iroh identity. A roster row is **never a data grant**.

The same live peer must also match an independent owner-signed, exact `VaultId/DocumentId/RevisionId/BLAKE3/NodeId/DeviceId` read grant, current independently witnessed grant epoch/digest, and device proof over a fresh challenge. The test uses the same secret seed for the Iroh endpoint and Ed25519 device proof; **no additional device private-key store**. Challenge consumption is delegated to an explicit `OneUseChallengeFence` trait, with a disposable in-memory test implementation; **production must provide a privileged server-held replay fence**. Only then may the adapter invoke the **existing Sync DAG** immutable verified read; it neither writes user Vaults nor creates a new sync engine.

## Negative gates

A real second Iroh peer presenting the same signed grant and device proof is denied because `Connection::remote_id()` differs; repeated challenge on the same session is denied. A fully signed, independently witnessed **Revoked** roster entry still cannot authorize the read. Stale roster epoch and different signed revision are denied without bytes. Tests use live loopback QUIC (no relay) and verified SQLite blobs; they cannot replace real VPS tests.

## Explicit missing production boundaries

- There is **no Node daemon handler** calling this adapter yet, no durable server-side owner/policy key custody, protected monotonic epoch witness, long-lived Linux Node Iroh identity, OS-protected replay store, or verified streaming/flow control. The output is a bounded lab Vec and never a public HTTP GET.
- This code **does not arm the Staya provider**: `serve-batch-gated` uses a separate sender-local, trusted test operator command. That approval must be integrated into a production authenticated provider before external reads. No plaintext staging file is eliminated here.
- [#572](https://github.com/Lalalalendia/Akula/pull/572) implements the Windows client identity store, **not Linux Node custody** or a pairing UI.
- The current test realm provisions test owner pins and challenge fence directly; no production grant provisioning or same-user-hostile storage assumptions are proven.

## Run

```bash
cargo test --manifest-path interop/akula-staya/Cargo.toml --features node-live-read-lab --test node_live_identity -- --nocapture
```

Added as a Linux step of the existing real Staya workflow; normal Windows/default builds have no new enabled feature. **Do not merge this stacked Draft until #531/#607 are accepted and all exact-head checks pass.** NODE-E2E-02/07 remain not accepted.
