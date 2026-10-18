# Akula Library: private iroh/QUIC laboratory

Independent research-only Rust 1.91 crate using **iroh = 1.3.0**. It does **not** change the Akula Sync Core. Designed to follow the previous UDP index lab (#227).

## Protocol and safety scope

- Two **real iroh endpoints** exchange QUIC streams over local UDP sockets (`127.0.0.1`), encrypted and authenticated by iroh endpoint ID. No simulation of QUIC.
- Explicit `presets::Minimal`: **no n0 DNS, no n0 relay, no address publication**, and no external discovery. The endpoint has its IPv4 socket explicitly bound to loopback. It therefore cannot assess WAN NAT traversal or relay fallback.
- Only an explicitly allowlisted receiver endpoint ID may request data. An unauthorized peer gets a closed connection. This is *not yet* a full signed bookshelf invitation or multi-device membership protocol.
- Files are test byte sequences held in memory. No arbitrary user paths, no file serving from a device, and no EXE/PDF/EPUB parsing.
- Protocol ALPN `akula-library/iroh-lab/1`, bounded binary manifest with 256 KiB parts, 16 MiB book cap, BLAKE3 of every part and complete book, request length cap. Valid/invalid byte envelopes verified by tests.
- App payload quota with a global and per-receiver counter. Reserves payload bytes before sending and does not refund after an aborted send (conservative). **Not a real cap on encrypted QUIC/IP wire bytes, CPU, mobile data, or relay costs**. Persistent accounting requires later implementation.
- Download resumes missing **verified in-memory** parts after interrupted connection. Not durable after process restart.
- Concurrency cap: 8 accepted connections. Per-stream timeouts and bounded receive buffers. No production DoS claims; stream-count rate limiting and server-side disk quotas remain to be added.

## Run

`cargo test --all-targets`

This invokes real loopback Iroh endpoints inside Tokio async tests. Tests include allowed client, untrusted peer, payload quota exhaustion, invalid manifest, invalid request, block corruption, resumed in-memory transfer and new UDP port under unchanged secret key.

## Acceptance criteria / next steps

1. Ensure `cargo test --all-targets` passes on GitHub Actions Rust 1.91 stable Linux. If CI fails, do not claim a working iroh test.
2. Create a separate process/client CLI with secure local key material, durable verified parts and signed BookCard, before testing distinct real hosts.
3. Test two real computers on different networks using an **explicitly configured, privately administered relay**. Compare direct vs relayed paths; packet capture must verify bytes and budgets.
4. Add persisted global/group/per-peer quota counters; 24-hour disconnect and key rotation tests; sender/receiver authorization and stop/continue flows.
5. Only then test Android/iOS background restrictions and Wi-Fi Aware. The current code does not connect physical devices.

Related Notion: https://app.notion.com/p/3fd32a84beec81a7af7cf0d1e32f3947