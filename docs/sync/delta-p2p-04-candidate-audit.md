# P2P-DELTA-04 — evaluate existing Rust transfer/delta components, do not fork another sync engine

Status: **research / laboratory only (2026-10-18)**. No production sender or receiver, no native or S3 feature activation, no File Gate commit, no trusted remote baseline, no changed V1/V2 wire or user files. The experiment in `experiments/p2p-delta-candidates` needs independent CI acceptance; measured bytes are **not** QUIC on-the-wire bytes.

## Context: our existing work

- `#270` and `#276` established text delta performance on 17 consecutive real Akula Markdown Git edits, and cold-base fetch costs on a disposable two-client MinIO fixture.
- `#281` (open when this audit was written) adds the opt-in Rust `AKTDLT01` codec, 2MiB cap and 128B/15% savings threshold; it intentionally does not replace `AKPKEXP1` nor activate S3 or desktop sync. Rust dictionary compression is not the same as CLI `zstd --patch-from`.
- P2P is a **different optional transport** built on pinned `calex794/roha@a42ad5aef1626df80d39398e05dac6e362bf6e49` -> `calex794/noq@2eabf9895d40c263aa3369afd66efcf4a5d95056`; see standalone `staya/roha-transport` branch and experimental Akula `#256`. Do not confuse the isolated QUIC/fragment tests with File Gate-authorized cross-device sync.
- On P2P, Akula may avoid sending a full immutable V1 blob *on the network* for each revision, provided its receiver reconstructs and independently validates complete canonical content and the entire authorized immutable revision DAG before any local publication. This is a future protocol change, not existing functionality.

## Available projects and adoption boundaries

| Candidate | What it actually does | Fits Akula | Recommendation |
|---|---|---|---|
| [iroh-blobs](https://github.com/n0-computer/iroh-blobs) 0.103.1 | QUIC blob / range / collection transfer with BLAKE3 verified content and partial downloads | Good for large binary objects, missing ranges, interruption/resume; **not** arbitrary byte-edit deltas | Isolated lab A/B against Staya's existing chunker, **no provider enabled by default** |
| [iroh-docs](https://github.com/n0-computer/iroh-docs) 0.101.0 | Signed namespaces/authors, key/value replica metadata, range-based set reconciliation; transfers content via separate blobs/gossip protocols | Valuable reference for compact inventory and diverging peer sets. Its author/namespace semantics do not equal Akula's VaultId/DeviceId/revision DAG | Study reconciliation algorithm, do not replace Akula Sync Core |
| [copia](https://github.com/paiml/copia) 0.3.0 | Pure-Rust rolling-checksum + BLAKE3 block signatures, delta and patch; also offers its own directory mirroring/SSH tool | Potential if sender **does not already have** receiver's full old bytes. It requires a receiver-to-sender signature exchange and explicit validation of all reconstructed bytes | Benchmark only `Sync::signature/delta/patch`, not its filesystem manager or conflict policy |
| [librsync-rs](https://github.com/mbrt/librsync-rs) | Rust bindings to C librsync signatures and patches | Same conceptual role as copia with FFI/dependency complexity | Not first choice |
| [fastcdc](https://github.com/nlfiedler/fastcdc-rs) 5.x | Deterministic content-defined chunking | Larger binary files / repeated content across boundaries, not efficient for tiny Markdown | Later binary A/B versus verified ranges; do not split every note |
| [zync](https://docs.rs/zync-rs/latest/zync/) 0.1.1 | zsync HTTP range transfer with optional bsdiff residue sidecars | Useful pattern: optional patch is never authoritative, fallback to full verified range | Inspiration only, not QUIC-native |
| [Automerge sync](https://docs.rs/automerge/latest/automerge/sync/) | Reliable-stream synchronization of Automerge CRDT changes and per-peer heads | Changes document semantics, merges, storage and auth model | Do not replace immutable Akula revision DAG merely to reduce traffic |

**Security discovery — urgent for an optional iroh-blobs adapter:** [iroh official security bulletin, October 8 2026](https://www.iroh.computer/blog/iroh-blobs-push-fix). Provider event mask incorrectly checked the `get` mode for other request kinds, permitting unsolicited `push` to fill disk. Affects 0.94.0–0.103.0; fixed in **0.103.1** and specified backports. Restrict *each* `get`, `get_many`, `observe`, `push` mode independently, verify peer grant before content operations, enforce quota, and keep PUSH disabled. Upstream's own README still marks 0.103.x as not production quality and recommends 0.35; 0.35 belongs to the older iroh API family, so **do not silently import it into pinned Roha 1.3**. Neither compatibility nor security is proven by a successful dependency resolver.

`iroh-blobs 0.103.1` Cargo depends on `iroh ^1.0` and optionally `noq ^1.0`; the semver constraints alone are not an integration test of our pinned Roha/noq forks. A compatibility spike must pin all three noq packages to the exact Akula SHA, commit Cargo.lock and run direct/relay/reconnect security tests.

## What DELTA-TEXT and copia optimize differently

- **Warm exact base on both peers**: use Akula `AKTDLT01` dictionary-Zstd with RevisionId/BLAKE3; no extra signature roundtrip. Favor the existing verified Core codec over a second compressor.
- **Receiver has a base, sender does not have those exact bytes**: copia can compute a delta using the receiver's block signatures; signature *and* delta bytes must be included in the cost, as well as one extra control exchange, CPU and resource budgets. A signature does not authenticate a peer or grant read/write permission.
- **No verified base**: send FULL-Zstd, not guessed sibling, not an on-demand remote base fetch solely to use a dictionary.
- **Large binary already partly held**: compare iroh-blobs verified range transfer to Staya's custom chunker. Content-addressed range reuse differs from reconstructing changed bytes.
- **Many divergent revisions**: peer inventory / compact set reconciliation is a separate metadata problem. Maintain exact RevisionIds, complete parent closure and tombstones; no probabilistic summary can act as an authority to drop history.

## Laboratory measurement now implemented

`experiments/p2p-delta-candidates` is a detached Rust crate with pinned direct candidate versions. Run it on a *complete* Akula Git checkout:

```sh
cargo test --manifest-path experiments/p2p-delta-candidates/Cargo.toml
cargo run --release --manifest-path experiments/p2p-delta-candidates/Cargo.toml -- .
```

It reuses the five first-party Markdown paths, adjacent Git commits and bounded 2MiB UTF-8 corpus from DELTA-TEXT-02. Refuses a non-representative run with <9 pairs or <3 paths. Each pair must roundtrip full Zstd, raw-dictionary Zstd and copia signatures/deltas. A separate bounded patch writer and independent BLAKE3 checks reject a changed base/target. It emits the full and dictionary bytes, serialized copia signature, serialized copia delta, sum of both directions, and coarse per-pair CPU encoding timings. No private note content is printed or transmitted.

**Important accounting limitations:** The crate uses `bincode` serialization as a temporary **size proxy**, not an approved, hardened P2P wire format. Each payload is assigned a hypothetical 120-byte envelope to compare against `AKTDLT01`; exact QUIC headers, FEC/retransmissions, authentication, negotiation, message framing, RTT and disk I/O are not included. Nor is this a native Akula codec A/B: its Zstd modes model the experimental codec's Zstd level and dictionary, not its exact encoder path. A green CI proves only the isolated local benchmark and tests.

The next stronger test must compare the *actual* `aquilum-core` `AKTDLT01` implementation from accepted #281 against `copia` and direct Roha endpoints. Do not report production P2P traffic savings yet.

## Gate order and integration plan

1. Merge #281 only after its current exact-head required CI passes; do **not** merge experiment-only transport files merely to get this benchmark.
2. Use P2P-DELTA-04's full two-way size accounting to decide whether copia adds value beyond dictionary Zstd. If no measurable improvement outside warm-base cases, reject the dependency.
3. Create a narrow Core-only codec interface for already-authorized exact revisions, with per-peer capability negotiation bound to the verified VaultId/DocumentId, immutable RevisionId, base and target BLAKE3, format version and 2MiB output limits. Existing `pub(crate)` codec requires explicit review before shared reuse. Do **not** move codec logic into Roha/noq.
4. Compare iroh-blobs 0.103.1 to existing 64KiB/16MiB Staya chunk framing on big binary and many-small-blob cases, with malicious pre-auth PUSH/get-many/observe, disk-capacity and cancellation regression tests. No new provider store until authorize/quotas pass.
5. Two independent Roha processes: old client compatibility, missing/wrong/stale/sibling base, 20+ offline generations, tombstones, sender/read-only grants, revocation, interrupted frame, resume, end-to-end payload/CPU/RTT/connection byte instrumentation.
6. Real two-PC LAN then WAN direct and explicit own-relay fallback. Preserve ordinary Akula local/S3 experience and existing File Gate gating; no automatic file apply without durable verified staging and an authenticated action.
7. A successful experimental transfer does **not** justify retiring canonical V1 immutable history, weakening root CAS, accepting an unauthenticated signature, or claiming installed production sync.

## Primary sources

- [Iroh-blobs protocol](https://docs.rs/iroh-blobs/0.103.1/iroh_blobs/protocol/index.html)
- [Iroh 2026-10-08 security bulletin](https://www.iroh.computer/blog/iroh-blobs-push-fix)
- [Iroh-docs range reconciliation](https://docs.rs/iroh-docs/0.101.0/iroh_docs/)
- [Copia pure Rust engine](https://docs.rs/copia/0.3.0/copia/) and [source](https://github.com/paiml/copia)
- [FastCDC](https://docs.rs/fastcdc/latest/fastcdc/)
- [librsync-rs](https://github.com/mbrt/librsync-rs)
- [Zync Rust](https://docs.rs/zync-rs/latest/zync/)
- [Automerge SyncDoc](https://docs.rs/automerge/latest/automerge/sync/)
- [Range-Based Set Reconciliation paper](https://arxiv.org/abs/2212.13567)
- [Akula #281](https://github.com/Lalalalendia/Akula/pull/281)
- [Akula #256](https://github.com/Lalalalendia/Akula/pull/256)
