# GNET-03: offline Roha LAN adapter (laboratory)

**Status:** code candidate. This is *not* a finished installed Akula game or
evidence of two physical computers connecting.

Independent experimental Cargo workspace at `modules/game-lan-roha`.
Depends on adjacent GNET-01/02 `game-session` and a pinned Roha commit
`1e4ce8aa9dcacd3d79d37d2a1ff803530d278b5d` (Iroh 1.3).
The main Akula Cargo.toml, Cargo.lock, UI and application remain unchanged.

## Strict LAN configuration

- `presets::Minimal`, no n0 DNS/relay service.
- Clear all address-lookup and relay providers and IP-default listeners.
- Disable UPnP/NAT-PMP/PCP port mapping.
- Disable extra captive portal/HTTPS latency checks.
- Bind only to a selected loopback/private/ULA/link-local local IP address.
- Connect only to a literal LAN IP:port plus an expected authenticated EndpointId.
- Never call `endpoint.online()`: it waits indefinitely when relays are disabled.
- Only after an explicit `host` or `join` command is a UDP listener created.

**Security boundary:** Roha authenticates the QUIC endpoint key, not its right
to enter a room. The host must **manually approve** by entering the guest ID
shown on the guest PC. Do not approve from the untrusted incoming ID alone.
GNET-04 must implement robust invitation, persistence, expiry and user UI.
No File Gate, Vault or operating system commands are exposed to game packets.

## Physical two-PC manual pilot

First, build while dependencies are available:

```sh
cargo build --manifest-path modules/game-lan-roha/Cargo.toml
```

In the same LAN, after turning off WAN access to the test machines and
recording a packet capture:

On the HOST computer, with a known actual LAN interface address:

```sh
cargo run --manifest-path modules/game-lan-roha/Cargo.toml --bin akula-game-lan-roha -- host 192.168.1.20:43000
```

Copy the printed HOST_ID and HOST_LAN_ADDR to the guest **out of band**.

On the GUEST computer, bound to its own LAN interface:

```sh
cargo run --manifest-path modules/game-lan-roha/Cargo.toml --bin akula-game-lan-roha -- join 192.168.1.30:0 192.168.1.20:43000 HOST_ID
```

Read the GUEST_ID from the guest terminal and enter the **full** matching ID
into the host prompt before any game frames are sent. Host drives 20
deterministic turns; guest sends its moves, checks each snapshot and returns
a duplicate last command as acknowledgement. The host ignores duplicates,
sends the authoritative final state, and waits for final confirmation.

Expect the `HOST PASS` and `GUEST PASS` messages on both sides.
Record: Roha SHA, game-session SHA, operating systems, bound IPs, firewall
behavior and pcap showing **zero external DNS/relay/UPnP/HTTP requests**.
Do not claim this until an actual two-machine trial has been performed.

## CI

`game-lan-roha-lab.yml` runs only when this lab path or its workflow changes,
separate from the main Akula workspace. It compiles and tests a loopback pair
with real Roha and bounded game frames, and checks no relay advertisement.
The exact Roha Git revision is pinned; transitive dependency lockfile has
not yet been committed and must be generated and reviewed after the first
successful dependency resolution before treating builds as reproducible.

## Limits

- IPv4 RFC1918 or locally scoped IPv6, not routed public addresses.
- Only two peers and a fixed 20-turn test, no general lobby/mDNS/GUI.
- No reconnect, replayed invitation defense, trust-on-first-use,
  client authoritative writes or host migration.
- The synchronous GameTransport interface is bridged by bounded channels
  with background QUIC stream tasks.
- No demonstrated WAN isolation without packet capture.
- First result is LOCAL LAB at most, not PHYSICAL 2-PC or INSTALLED USER.

Spec: https://app.notion.com/p/40132a84beec8162aa47e0ef42e999fa
