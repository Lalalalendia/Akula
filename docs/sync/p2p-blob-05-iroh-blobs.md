# P2P-BLOB-05 — pinned iroh-blobs 0.103.1 versus existing Staya/Roha framing

**Scope: disposable laboratory only.** No changes to Akula desktop, Sync Core, S3 V1/V2, File Gate or user data. Stacked on the already isolated `feat/staya-roha-transport-01` branch, not on main. Not a request to merge the unfinished Staya adapter.

## Why

Our existing Staya `AKSTAY01` frame has a 92-byte header, bounded 64KiB chunks, full BLAKE3 and per-fragment BLAKE3 with an application-level 8-byte receipt per frame. The direct Roha/noq tests pass but the receiver is in RAM and does not have durable File Gate acknowledgment. [`iroh-blobs` 0.103.1](https://docs.rs/iroh-blobs/0.103.1/iroh_blobs/) has verified Bao/BLAKE3 range requests, multi-blob collections and potential partial-file resumability. We must compare them without creating a second revision DAG or accepting a provider store into Akula.

**IMPORTANT**: `iroh-blobs 0.103.1` README describes it as not yet production quality. The recommended stable 0.35 belongs to an older API family and is not a drop-in replacement for pinned Roha 1.3.

### Security vulnerability and defense

Iroh's [official 2026-10-08 advisory](https://www.iroh.computer/blog/iroh-blobs-push-fix) documents an incorrect per-request `EventMask` dispatch affecting 0.94.0–0.103.0; an incoming unauthorized `push` could be processed because the handler checked `mask.get`, potentially filling local disk. The 0.103.1 fix uses the correct mask for each request type.

In 0.103.1:
- `RequestMode::None` **allows** the request without event notification. It is NOT deny.
- `RequestMode::Disabled` explicitly rejects `push` and `get_many` before handling.
- `ObserveMode` only exposes `None/Notify/Intercept`, so `None` is **not disabled**. For this Staya laboratory, use `ObserveMode::Intercept` and unconditionally reject requests. Do not trust a default EventMask as a blanket read-only authorization.
- Use `ConnectMode::Intercept` and an explicit allowed EndpointId; `GetRequestReceived` only grants an explicit BLAKE3 hash and single raw blob. Reject hash sequences, foreign hashes and unknown EndpointIds.
- These controls **do not** constitute product VaultId/DeviceId pairing, signed per-directory read grants, anti-replay, pre-handshake ingress filtering, disk quota, or File Gate access; those must be added before product adoption. Never publish a public `BlobTicket` or use `presets::N0` in Akula by default.

Lab test `a_get_can_be_allowed_without_enabling_push_or_other_reads` performs an actual direct QUIC GET while trying: unauthorized hash GET, get-many bypass, observe, unsolicited PUSH followed by a check that the target MemStore remains `NotFound`, and an unrecognized peer key. All probes have deadlines. An upstream negative PUSH may appear to succeed on the send side; our accepted criterion is **zero bytes/entries in recipient store**.

### Fork compatibility

The laboratory Staya crate already pins `iroh = git calex794/roha@a42ad5a...` and `noq/noq-proto/noq-udp = calex794/noq@2eabf989...`. `iroh-blobs 0.103.1` itself declares a crates.io `iroh ^1.0` dependency. The new **lab-only** `[patch.crates-io].iroh` ensures it resolves to Roha, not an *additional* upstream iroh in parallel. CI runs `cargo metadata` to enforce exactly one pinned package for each `iroh`, `noq`, `noq-proto`, `noq-udp` and exactly one `iroh-blobs 0.103.1`. A successful constraint resolution is NOT sufficient without a compiled network regression.

## Real QUIC A/B benchmark and accounting

`tests/iroh_blobs_compare.rs` sends independently verified deterministic inputs of **1 byte, 64KiB+17 bytes, 1MiB and 8MiB** through two isolated direct-only Roha/noq endpoints and `iroh-blobs` 0.103.1. For Staya it uses the **actual existing** `TransferManifest`, `encode_chunk`, `VerifiedAssembly` and per-chunk QUIC receipt. For blobs it uses `get_blob` with upstream Bao validation. Both restore the exact full bytes.

Outputs from the two methods are **different protocol counters**, not measured equivalent full network traffic:
- `staya_application_bytes_including_receipts` sums the exact 92B+data frames and 8B app receipts, excluding QUIC/UDP/IP.
- `blobs_receiver_protocol_bytes` uses `Stats::total_bytes_read` on the receiver (including blob response framing/proofs as measured by upstream), not symmetric to Staya's sent+received app bytes and not complete UDP traffic.
- Both log elapsed transfer microseconds after connecting, on **one local CI host**. Connection establishment, data import, TCP/UDP/IP/QUIC overhead, real WAN latency, disk staging and grant/auth policies are not measured.
- Existing Staya's per-frame application ACK will naturally penalize it on high-bandwidth links; results are a design comparison, **not** a fair broad throughput verdict.

The first goal is **ABI/API compatibility + security refusal + byte-exact local transfer**. A future controlled A/B needs per-direction QUIC/UDP byte counters from the same measurement layer and loss/RTT shaping, partial-range resume and interrupted mid-stream tests. Only if it materially improves reliability and user-visible transfer cost should an optional product adapter be considered.

## CI and acceptance

The new `.github/workflows/staya-iroh-blobs-lab.yml` uses read-only GitHub token, pinned toolchain, exact forks and machine-checkable metadata. It runs direct tests and preserves generated Cargo.lock, logs and rustfmt differences. Until the exact generated `Cargo.lock` is committed and a `--locked` clean-checkout CI is green, **do not merge or claim reproducible dependency closure**.

Required to promote beyond this lab: reproducible cross-platform builds with pinned deps, malicious read/write tests with disk quotas, unsafe-remote denial and cancellation, real two-PC LAN/WAN tests, direct/relay path attribution, durable staging after restart, independent approved grants, File Gate commit and rollback. Prohibit any implicit migration of canonical S3 V1/V2 or local Akula history.

## Sources

- [Iroh 0.103.1 API and production warning](https://docs.rs/iroh-blobs/0.103.1/iroh_blobs/)
- [Iroh push/access-mask security fix (2026-10-08)](https://www.iroh.computer/blog/iroh-blobs-push-fix)
- [Iroh-blobs request mode implementation](https://github.com/n0-computer/iroh-blobs/blob/main/src/provider/events.rs)
- [Iroh-blobs historical PUSH negative tests](https://github.com/n0-computer/iroh-blobs/blob/main/src/tests.rs)
- [Current Akula Staya framing](https://github.com/Lalalalendia/Akula/blob/feat/staya-roha-transport-01/staya/roha-transport/src/transfer.rs)
- [Previous P2P-DELTA-04 numerical comparison](https://github.com/Lalalalendia/Akula/pull/294)
