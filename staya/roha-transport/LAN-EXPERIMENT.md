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

## Two physical computers: acceptance checklist (NET-DIRECT-13b)

The pre-built **laboratory** executables are available as GitHub Actions artifacts for seven days, for successful checks on this branch: `staya-lan-windows-x86_64-lab` and `staya-lan-linux-x86_64-lab`. They are not an installer or production build. Each bundle contains an executable and its SHA-256 digest. Verify the digest before use, and do not run binaries from failed or unknown workflow revisions.

### Setup

1. Place Akula-1 and Akula-2 on the **same trusted LAN**, preferably wired to the same router or switch. Turn off Wi-Fi client isolation. Keep the local network working while disconnecting the router's WAN uplink. Do not turn off the local router or DHCP service.
2. Create a separate, private working directory on each PC, and **do not place key files in a shared folder**. On Windows use a private directory restricted to your user account; the laboratory CLI does not yet enforce Windows ACLs. Do not reuse these experimental keys for real content.
3. Run `staya-lan gen-key-file ./akula1.key` on A and `staya-lan gen-key-file ./akula2.key` on B (use `staya-lan.exe` on Windows). Write down **only** their displayed public endpoint IDs, and exchange them by a trusted channel. Never exchange `*.key`.
4. Verify both PCs have LAN IPv4 addresses and can exchange ordinary packets. Permit *inbound UDP 42424 only from A's LAN IP* to B; permit mDNS UDP 5353 on the trusted local interface as needed. Do not forward ports through the internet-facing router.

### Test L01: manual direct QUIC without internet

On B: `staya-lan serve @./akula2.key A_ENDPOINT_ID 0.0.0.0:42424`.
On A: `staya-lan ping @./akula1.key B_ENDPOINT_ID B_LAN_IPV4:42424`.

Pass condition: A displays `direct_lan_ping_ok`, B displays `ping_ok`, relay remains disabled. Record both operating systems and both LAN IP ranges, but omit precise personal IPs from any public report.

### Test L02: automatic discovery → QUIC

Stop B's manual server, then on B start:
`staya-lan serve-mdns @./akula2.key A_ENDPOINT_ID 0.0.0.0:42424 B_LAN_IPV4`.

On A (knowing only the **already authorized** B EndpointId, not its current IP):
`staya-lan ping-discovered @./akula1.key B_ENDPOINT_ID 10`.

Pass condition: A displays `direct_lan_ping_ok peer=... via=mdns relay=disabled` and B displays `ping_ok` with the same peer; traffic is direct. A discovery response alone is **not** sufficient.

### Negative tests and fallback

- With an incorrect B EndpointId on A, neither a manual nor automatic ping may succeed.
- With a different A identity not in B's allowlist, the connection must fail; this test does not grant file access.
- Disable multicast on the LAN or block mDNS on a test network: `ping-discovered` must fail safely; manual `ping` by the known direct IP must still work.
- Block inbound UDP 42424 on B: connection must fail in bounded time. Restore the firewall afterward.
- Restart both endpoints using the **same protected local key files**, then repeat the direct test. Never delete a real synchronized file during laboratory tests.
- Record the exact Git commit, artifact SHA256, operating systems, test name, exit codes, presence/absence of relay, firewall state, and whether WAN was disconnected. Report success only if both endpoints exchanged QUIC PING/PONG.

**Not yet demonstrated:** cross-provider NAT traversal, two true physical PCs, safe product-grade enrollment and key storage, or file synchronization. None of these is implied by virtual network-namespace success.

## Experimental public contact exchange — NET-DIRECT-13c

This is **not automatic pairing** and does not implement PAKE or a short SAS code. A public contact can be forged; no file permissions are granted.

On B: `staya-lan show-contact @./akula2.key > akula2.contact`. Send the public `akula2.contact` through a trusted channel. On A: `staya-lan inspect-contact akula2.contact`. Before allowing a connection, independently verify B's **full public EndpointId** with B's operator. Then run `staya-lan confirm-contact akula2.contact VERIFIED_FULL_ENDPOINT_ID > akula2.trusted`. A mismatch is rejected. The trusted record is a local audit of human approval; current `serve` and `ping` commands still use pre-enrolled IDs. There is **no automatic enrollment**, no vault access, and no claim that comparing a full ID is a usable final user experience. The future one-time invitation/PAKE/SAS flow must be separately implemented and audited; never invent a cryptographic short code by truncating a public key.
