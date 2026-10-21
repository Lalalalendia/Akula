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
