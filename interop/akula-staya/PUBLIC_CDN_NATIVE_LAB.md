# Native S3/P2P reader laboratory (non-shipping)

This is a **feature-gated adapter within the EXISTING** `interop/akula-staya` crate. It does not change Akula's shipping Rust workspace, app UI, Sync Core, File Gate, OS peer enrollment, relay or updater.

## What is implemented

- Parse the exact existing browser manifest `version=1`, `content_id` and `chunks[].sha256` (SHA-256).
- Require an **independently trusted** SHA-256 digest of the raw manifest before parsing. This is an input trust assertion; **publisher signature validation is NOT implemented**.
- Bound total object size, chunk size, offsets, chunk count, manifest bytes and metadata.
- Prefer local chunk cache, then optional existing Iroh/Staya source, then read-only S3 HTTPS Range provider. Every received part must match the manifest's byte length and SHA-256.
- Fail closed for wrong `206`, `Content-Range`, byte lengths and content-encoding. Separate counters for useful cache/peer/origin bytes and network bytes.
- Stream one verified part at a time to a **caller-owned private temporary writer**. Publish nothing until final SHA-256 match; calculate separate *BLAKE3* for Staya's `ContentId`.
- Expose `ExistingStayaIrohPublicReader` + `ExistingIrohPeerSource`: takes an already authenticated `iroh::endpoint::Connection`. It does NOT implement a new QUIC ALPN, a sender service, an inbound GET grant, or a bypass around existing security.

## Build/test

```sh
cargo test --manifest-path interop/akula-staya/Cargo.toml \
  --features public-cdn-reader-lab --lib public_cdn_lab
```

This will resolve the existing pinned `staya-transfer` revision and Rust toolchain; do **not** regenerate the shipping `aquilum-app/Cargo.lock`. Tests use only synthetic public bytes and in-memory sources. No S3 keys, AWS traffic, private files, OS peer keys or network ports are needed.

## Distinct identities (critical)

The browser's `ContentID` = SHA-256(full object). Staya/Bao uses BLAKE3(full object). Identical bytes produce both hashes, which can be linked only *after* complete verified streaming. An S3 ETag is not a substitute for either.

## NOT implemented / separate gates

1. A real Staya/Bao provider for public read. The trait is an integration seam only; a real Staya provider still needs consent policy, relay/traffic limits, peer availability, bounded wire parsing and E2E checks. The identity of the transport peer alone is not a read permission for private objects.
2. A real network S3 adapter enforcing response length **during streaming**, host allowlists, conditional version pinning, TLS and cancellation. `OriginReply` is a bounded response contract for tests, not a production HTTP client.
3. Publisher Ed25519 signature verification / trusted publisher key discovery, and signed catalog updates.
4. Atomic on-disk staging and user-facing publication. The generic writer is caller-owned; a failed full digest leaves only an untrusted temporary output and returns no verified receipt.
5. Native↔browser interoperability: WebRTC and Iroh use different transports and require an explicit gateway or interoperable exchange protocol.
6. Actual QUIC peers, remote NAT, real S3, full 1 GiB browser performance, and a shipping Akula feature.

## Testable next vertical slice

- Reuse existing Iroh connection and peer pin/identity gates.
- Add a **read-only, permission-scoped** public chunk provider on the existing Staya transport; reject private Vault roots and unsolicited peer requests.
- Adapt the verified S3 range reader to the provider's existing SDK/HTTP transport.
- Prove first 64 MiB on sender A from S3, second client B partly/entirely from A, independent full SHA-256 + BLAKE3 and origin-byte reduction; then test peer offline/corrupt, revocation, NAT/direct/relay.
- Keep the native lab outside normal builds until exact-head CI and physical-device acceptance.

Canonical Notion scope: https://app.notion.com/p/40332a84beec81c0a1bbd9e5c02c7de0
