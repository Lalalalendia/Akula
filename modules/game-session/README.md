# GNET-02 — in-memory transport and bounded game frames

This directory remains a zero-dependency, standalone Rust crate. GNET-01
implements authoritative Connect Four rules. GNET-02 adds:

- `GameTransport`: transport-agnostic interface for authenticated sender
  identity, sending frames, and nonblocking receive polling.
- `InMemoryTransport`: two ends linked by bounded queues, no network access.
  The sender identity is assigned by the endpoint, never parsed from a packet.
- `WireMessage`: versioned fixed-size binary command (37 bytes) and state
  snapshot (79 bytes) frames. Bad magic, version, length, game state, turn
  parity, gravity, or outcome is rejected.
- 20-turn scenario crossing two in-memory endpoints and verifying both players'
  identical game state after each turn. Repeat frames cannot duplicate moves.

## Invariants

- PeerId must be established by an authenticated real transport on GNET-03.
- Room ID is a routing token, never an access secret. Admission and revocation
  are GNET-04, not implemented here.
- A client must trust snapshots only from the admitted host and reject stale
  revisions. This will be enforced by the GNET-04 client controller.
- This is NOT a working LAN transport; no sockets, OS network permission,
  browser, Bluetooth, persistence, or cross-device acceptance is included.
- The entire crate lives under `modules/game-session` and intentionally does
  not change Akula's expensive root Rust workspace or its Cargo.lock.

## Focused verification

From repository root:

```sh
cargo fmt --manifest-path modules/game-session/Cargo.toml --all -- --check
cargo test --manifest-path modules/game-session/Cargo.toml --locked --offline --all-targets
cargo clippy --manifest-path modules/game-session/Cargo.toml --locked --offline --all-targets -- -D warnings
```

CI scope is limited by `.github/workflows/game-session-isolated.yml`.

Design and feature status: https://app.notion.com/p/40132a84beec8162aa47e0ef42e999fa
