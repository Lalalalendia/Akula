# NET-DIRECT-13a: Akula-1 <-> Akula-2, direct LAN

Isolated experiment using the repository's Roha -> noq fork chain; no cloud, relay, public discovery, file writes, or Akula Sync Core.

## Build

From `staya/roha-transport` with Rust 1.91 or later:

```sh
cargo run --bin staya-lan -- gen-key-file ./akula1.key
```

Run on both machines with different local key file names. Each command prints a public `endpoint_id` only. On Linux/macOS the key file is created with owner-only permissions (0600), and existing files are never overwritten. Keep the key out of Git, logs and backup uploads. On Windows file permissions require manual review because this prototype does not yet enforce an ACL.

Akula-2 (server at LAN IP `192.168.1.22`):

```sh
cargo run --bin staya-lan -- gen-key-file ./akula2.key
cargo run --bin staya-lan -- serve @./akula2.key AKULA1_ENDPOINT_ID 0.0.0.0:42424
```

Akula-1:

```sh
cargo run --bin staya-lan -- ping @./akula1.key AKULA2_ENDPOINT_ID 192.168.1.22:42424
```

Allow **inbound UDP port 42424** to Akula-2 from Akula-1's LAN IP using an operating-system firewall rule; never expose the port to untrusted networks. Both PCs must be on the same routed LAN (client isolation disabled). Stop server with Ctrl-C.

An expected successful client message is `direct_lan_ping_ok`; the server also prints `ping_ok`. The transport deliberately has no relay or external discovery services, so success proves a QUIC exchange over a reachable direct UDP socket. This alone does not prove Internet NAT traversal, authorization for files, or identity verification by a human. This is a manually configured *pre-enrolled* endpoint allowlist, not a complete pairing protocol.


## Optional trusted-peer automatic discovery (NET-DIRECT-13b)

The manual IP commands above remain the guaranteed fallback. mDNS is optional, broadcasts the public endpoint ID and is unsuitable for public or untrusted Wi-Fi.

Akula-2, using its actual LAN IPv4 address (not 0.0.0.0 as the advertised address):

```sh
cargo run --bin staya-lan -- serve-mdns @./akula2.key AKULA1_ENDPOINT_ID 0.0.0.0:42424 192.168.1.22
```

Akula-1, with the previously enrolled Akula-2 endpoint ID:

```sh
cargo run --bin staya-lan -- ping-discovered @./akula1.key AKULA2_ENDPOINT_ID 10
```

Discovery runs for at most 20 seconds and accepts only records with the enrolled endpoint ID. A discovered IP address is **not** trusted: QUIC must still authenticate the enrolled key, and nothing is authorized to read or modify files. When multicast is blocked by network segmentation or the OS firewall, use the manual `ping` command. This first implementation supports IPv4 discovery and requires a pre-enrolled peer: automatic pairing, IPv6 scope identifiers, and real physical-LAN acceptance remain separate tasks.

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
