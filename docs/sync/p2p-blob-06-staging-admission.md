# P2P-BLOB-06 — bounded pre-admission to staging (isolated lab)

**Status: research-only; stacked on draft #301.** Does not install a product receiver or modify Akula Sync Core, File Gate, S3 V1/V2, user documents or either pinned Roha/noq fork.

## Motivation and threat boundary

The successful [P2P-BLOB-05 SIGKILL test](https://github.com/Lalalalendia/Akula/pull/301) restored integrity, but recovered zero verified bytes from crashed FsStore and re-fetched the full 8,388,621-byte file. A hostile peer must not cause unbounded disk occupancy or parallel downloads.

Staya's existing 16MiB frame limit protects only its own AKSTAY01 framing, **not** a separate iroh-blobs GET/FsStore path. Upstream [FsStore Options in v0.103.1](https://github.com/n0-computer/iroh-blobs/blob/v0.103.1/src/store/fs/options.rs) has inline thresholds, batching and optional GC but **no hard disk quota**.

## Bounded in-memory admission ledger

The new independent Rust module in staya/roha-transport/src/admission.rs requires known, independently trusted metadata **before a QUIC GET or FsStore writer is started**. The laboratory synthesizes this metadata; a production Akula integration must first verify VaultId, DeviceId bound to EndpointId, exact RevisionId, expected BLAKE3 and byte count. Neither hash nor length from a remote packet authorizes storage.

StageBudget atomically tracks, under one Mutex: one-object byte maximum, global and per-EndpointId reserved bytes, global and per-endpoint active slot counts, and duplicate digests. Integer overflows and zero/invalid configurations are denied. Mutex poisoning closes admission. Leases cannot be cloned; cancellation or panic drops a lease and releases its in-memory accounting. Verified objects **remain charged** as long as their lease exists; a caller must delete temporary backing data **before** dropping the lease. Verification checks exact byte length and full BLAKE3, and does **not** grant File Gate authority.

| Bound | Denial behavior |
|---|---|
| max_object_bytes | Reject oversized or implausibly large single object |
| max_tracked_bytes | Reject oversubscription across all admitted objects |
| max_peer_bytes | Stop one endpoint exhausting all reserved bytes |
| max_active | Stop global parallel admission explosion |
| max_peer_active | Stop single-endpoint parallel admission explosion |
| duplicate hash | Refuse concurrent reservation of identical digest |

Laboratory unit tests cover invalid/zero limits, huge size and overflow, duplicate hash, global and per-peer byte/slot ceilings, corrupted target verification, retaining capacity until cleanup, release on task failure and 12 concurrent contenders with at most 3 admitted leases.

A separate QUIC test in staya/roha-transport/tests/iroh_blobs_compare.rs denies oversize and competing admissions **before sending further GET requests**, then fetches exactly one approved blob over pinned Roha/noq, checks the bytes and hash again, and accounts the completed staged object until cleanup.

## What this does NOT solve

**This preflight ledger is NOT a hard physical filesystem quota.** The state is process-local and loses accounting across SIGKILL. It does not cap Bao/outboard/DB size, temporary write amplification, unrelated writers, total retained disk occupancy after an untracked restart, file-count explosion or per-process memory beyond the declared object count. Neither a completed QUIC request nor a zero-checked in-memory ledger proves durable capacity. These require an OS-enforced bounded staging filesystem, a persistent trusted ledger or startup reconciliation and fail-closed recovery.

It is also not peer enrollment, signed VaultId/DeviceId grants, File Gate authorization, S3 protocol change, real WAN/relay or two-machine user sync. Retain draft #301's negative tests of unauthorized GET, GET-MANY, OBSERVE and PUSH; no new incoming provider is enabled.

## Acceptance and next gate

The PR remains a stacked **draft** on the P2P-BLOB-05 experimental branch. It adds no external dependency; the existing pinned Cargo.lock remains authoritative. Require green exact-head unit tests, a real admitted QUIC transfer and no regressions in SIGKILL, reconnect, peer refusal, rustfmt and docs.

Next: implement and independently test an OS-enforced bounded staging volume plus restart reconciliation, then run physical two-client LAN/WAN/own-relay trials. Neither a successful lab run nor an in-memory reservation permits product deployment.
