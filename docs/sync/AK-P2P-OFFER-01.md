# AK-P2P-OFFER-01 — bounded offer/consent contract

**Status:** experimental Rust Core, opt-in, metadata-only. No installed UI, network, Iroh transfer, key enrollment, grants, filesystem access, File Gate, or durable replay guarantee.

Source: [Notion P2P-RESEARCH-20](https://app.notion.com/p/3fe32a84beec81fbbbabc65aaee53ac7). Separate draft UI: [Akula #513](https://github.com/Lalalalendia/Akula/pull/513). Manual transport: [Staya #67](https://github.com/Lalalalendia/staya-transfer/pull/67).

## Contract

1. **OfferV1:** typed source/recipient Akula DeviceId + Iroh EndpointId, offer UUID, 128-bit nonce, creation/expiration, 1–8 selected immutable file roots and bounded metadata.
2. **Limits:** at most 8 MiB per file, 32 MiB total, 10 minutes lifetime. Unsafe or ambiguous display names rejected, never sanitized and never used as target paths.
3. **Discovery is not trust:** the caller MUST supply independently validated bindings (actual QUIC-authenticated peer plus trusted Akula enrollment) for both identities. Taking them from untrusted offer JSON or mDNS completely defeats the boundary.
4. **Consent:** the receiver can accept a unique sorted subset or decline. Each DecisionV1 binds exact offer ID and a domain-separated fixed-field BLAKE3 manifest commitment.
5. **One-shot in-memory sender session:** Accept / Decline / Cancel / Expire are terminal; wrong peer, extra JSON authority fields, wrong root/size, duplicate selection and repeated decision fail closed.
6. **SelectionDraft is not TransferGrant:** this module contains no method to open local files, send bytes, modify the vault, or issue positive authorization.

## Critical production gates NOT implemented

- A real signed or independently enrolled Akula DeviceId-to-EndpointId binding, including key rotation and revocation.
- A persistent, crash-safe anti-replay fence keyed on offer ID, nonce, epoch and verified peer; current in-memory suppression does not survive restart.
- Clock/trusted freshness and version compatibility rules beyond this lab schema.
- One narrow Akula-authorized capability for the **exact** immutable root set. Recheck rights before every new Staya GET/range and before any File Gate action.
- Secure pinned source handle, root/length verification, Windows path races and owner-only staging, durable FsStore supervisor and independent Bao physical readback.
- Bounded metadata-only preview, real trusted-peer selection and receiver dialog in Masonry, user-visible direct/relay diagnostics, true verified progress.
- Physical Windows principals, LAN and Internet/NAT two-PC acceptance. [Staya hardware gate #51](https://github.com/Lalalalendia/staya-transfer/issues/51) remains HOLD.

## Validation

GitHub Actions runs Rustfmt, opt-in core tests and Clippy. All tests use synthetic metadata and operate without network, secrets, file reads or writes. Exact-head status must be checked before merge; no auto-merge.

Target: cargo test --locked -p aquilum-core --features p2p-offer-lab p2p_offer_lab::tests

Next integration: add **real policy-issued** bounded TransferGrant and a fail-closed adapter between the offer state and the existing Staya verified batch. Do not connect a draft selection directly to the Staya sender.

## AK-P2P-GRANT-JOIN-02 — current RN-4C ledger compatibility

The older [RN-4C / RN-4E1b-03a grant research #433](https://github.com/Lalalalendia/Akula/pull/433) already has an opt-in Core SQLite grant model; **do not implement a second grant issuer**. It holds **one active grant per (VaultId, DeviceId)** and uses `MAX_LAB_GRANT_BYTES = 4 MiB`. An Akula OfferV1 permits 8 files of 8 MiB each. These are incompatible as a direct batch grant.

An additional **test-only** source file `p2p_offer_grant_join_lab.rs` mirrors the public `StoredPull` field shape **without importing #433's unmerged code**. It compares consent selection to one observed grant: exact known sender/receiver, real-peer-id-shaped fixture, local VaultId, one immutable content root + size, policy issuer + epoch, nonce and expiry. A matching fixture returns only `ConsistentOnlyNoAuthorization`; it grants no GET and cannot invoke File Gate.

**Negative tests:** two selected files, over-4-MiB selected object, spoofed sender/receiver/QUIC peer, foreign vault, changed recipient, wrong root/length, mismatched nonce, stale epoch, untrusted issuer, invalid expiry and post-accept offer mutation.

**Limits:** local identities, observed grant, issuer and epoch are all injected *test fixtures*. This does **not** exercise actual SQLite, signature checks, OS Broker, current revocation, sealed issuer, network or file bytes. The source `SelectionDraft` remains ordinary in-process data and is not a capability. Do not promote this consistency check to authorization or a production API.

**Required next step:** marry the actual Core/Broker-controlled `StoredPull` to a QUIC authenticated remote identity, minted durable recipient consent, and short grant fencing. Align multi-file roots with a genuinely revised grant model (atomic root-set or per-root serial grants with durable nonce/replay handling), while preserving the one active grant invariant until migrated and reviewed. No longer-lived grant than the offer, no network reads before explicit consent.
