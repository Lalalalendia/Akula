# GNET-01: Connect Four game session

Small, deterministic Rust package with no external dependencies, sockets or Akula file access.
Not yet a network game, installed feature, or complete room service.
The main aquilum-app workspace and lockfile are untouched.

## Contract

- Host plays Red; invited guest plays Yellow; only these identities have a view or can move.
- The caller MUST take the PeerId from the authenticated transport, not from a message payload.
- RoomId is not a password. GNET-04 must generate room IDs securely, admit peers,
  manage invitation secrets, and ensure persistent increasing room epochs.
- Sequences start at 1, strictly increase per accepted command and per player.
- Repeated last accepted command has no effect; different payload at same sequence fails.
- Replays from previous epochs or other rooms fail. Invalid moves do not advance sequence.
- Player snapshots include the state, status, revision, identity of the seat, and own next sequence.
- No network, persistence, transport, file permission or rejoin implementation is included.

## Verification

Run from repository root:

    cargo fmt --manifest-path modules/game-session/Cargo.toml --all -- --check
    cargo test --manifest-path modules/game-session/Cargo.toml --locked --offline --all-targets
    cargo clippy --manifest-path modules/game-session/Cargo.toml --locked --offline --all-targets -- -D warnings

The path-filtered isolated workflow runs only this small package. Required Application checks
still reports its stable policy gate, but modules/game-session does not enter main Rust workspace tests.

Next: GNET-02 in-memory transport and bounded message protocol; GNET-03 Roha LAN adapter;
GNET-04 secure admission; GNET-05 physical two-PC acceptance with WAN disconnected.

Source requirements: https://app.notion.com/p/40132a84beec8162aa47e0ef42e999fa
