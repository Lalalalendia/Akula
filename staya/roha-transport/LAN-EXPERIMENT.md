# NET-DIRECT-13a: Akula-1 <-> Akula-2, direct LAN

Isolated experiment using the repository's Roha -> noq fork chain; no cloud, relay, public discovery, file writes, or Akula Sync Core.

## Build

From `staya/roha-transport` with Rust 1.91 or later:

```sh
cargo run --bin staya-lan -- gen-key
```

Run that on **both PCs**, recording each machine's endpoint ID and secret key **privately**. Secrets printed by `gen-key` are for laboratory use only; they are exposed in terminal output/process arguments. Do not reuse them as production keys or share logs containing them.

On Akula-2 (server), for example with LAN address `192.168.1.22`:

```sh
cargo run --bin staya-lan -- serve B_SECRET_HEX A_ENDPOINT_ID 0.0.0.0:42424
```

On Akula-1:

```sh
cargo run --bin staya-lan -- ping A_SECRET_HEX B_ENDPOINT_ID 192.168.1.22:42424
```

Allow **inbound UDP port 42424** to Akula-2 from Akula-1's LAN IP using an operating-system firewall rule; never expose the port to untrusted networks. Both PCs must be on the same routed LAN (client isolation disabled). Stop server with Ctrl-C.

An expected successful client message is `direct_lan_ping_ok`; the server also prints `ping_ok`. The transport deliberately has no relay or external discovery services, so success proves a QUIC exchange over a reachable direct UDP socket. This alone does not prove Internet NAT traversal, authorization for files, or identity verification by a human. This is a manually configured *pre-enrolled* endpoint allowlist, not a complete pairing protocol.

## Tests

```sh
cargo check --all-targets
cargo test --all-targets
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

### Acceptance to be done on two physical PCs
- Internet disconnected but LAN operational: ping succeeds, no relay.
- Wrong endpoint ID/key: no successful ping.
- Wrong allowed peer: no successful ping.
- Firewall UDP blocked: bounded error instead of a false success.
- IP address changes: manually update the address and reconnect.
- Collect p50/p95 connection time in future tests; no benchmarks claimed yet.

## Scope

No persistent key vault, QR enrollment, discovery, file access, multi-device group, NAT punch, dynamic address selection or automatic sync. These belong to NET-DIRECT-13b through 13g. CI executes isolated process tests; tests involving different physical computers still require a separate report.
