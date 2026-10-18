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
- Additionally, both methods now snapshot **the same pinned Roha/noq `Connection::stats().udp_tx/udp_rx` counters**, before and after the transfer, on the sending/requesting connection: bytes and datagrams, counted separately by direction. This provides comparable UDP-datagram byte deltas within the exact same QUIC implementation and avoids presenting the above asymmetric application counters as interchangeable. These UDP stats do not include IP/UDP headers, unrelated endpoint/discovery traffic or all physical NIC overhead.
- Both log elapsed transfer microseconds after connecting, on **one local CI host**. Connection setup and importing the bytes happen before snapshots. Real WAN RTT/loss, relay, File Gate, durable staging, grants and user-experience timings are not measured.
- Existing Staya's per-frame application ACK can penalize it on high-delay paths, while iroh-blobs has Bao proof overhead. Interpret the same-layer UDP measurements as **specific to this fixture** (one host, sequential uploads and one request per blob), not a broad throughput verdict.

## First measured accepted UDP A/B (2026-10-18)

[CI #39153129204](https://github.com/Lalalalendia/Akula/actions/runs/39153129204) passed: existing Staya regressions plus new authorization tests, including unknown peers and denied PUSH/GET-many/observe.

Both sides count *the same Roha/noq UDP datagram statistics* `Connection::stats()` for request/sender endpoints, tx+rx combined after connection establishment:

| Content | Staya UDP tx+rx bytes | iroh-blobs UDP tx+rx bytes | Staya datagrams total | iroh-blobs datagrams total | Staya transfer us | blobs transfer us |
|---:|---:|---:|---:|---:|---:|---:|
| 1 B | 10,973 | 8,573 | 18 | 16 | 3,705 | 4,151 |
| 65,553 B | 74,779 | 78,974 | 63 | 65 | 7,333 | 7,194 |
| 1 MiB | 1,083,289 | 1,088,649 | 821 | 769 | 56,436 | 42,815 |
| 8 MiB | **8,620,324** | **8,630,250** | **6,501** | **6,014** | **445,109** | **300,930** |

`iroh-blobs` finished the 8MiB transfer ~32% faster in this single local runner sample and used ~487 fewer datagrams, while requiring ~9,926 more counted UDP payload bytes (+0.115%). Not representative of WAN RTT, relay, losses, Android battery or production. **Do not claim universally faster.** Big speed difference may be Staya's blocking application ACK once per 64KiB frame, not just Bao itself. Measure Staya's batched/streamed ACK policy separately before replacing implementation.

**Verified:** [CI #39153699873](https://github.com/Lalalalendia/Akula/actions/runs/39153699873) passed `resume_missing_verified_ranges_after_new_quic_connection`: receiver fetched exactly 65,536 verified bytes of a 2,097,165-byte blob, closed the first QUIC connection, used locally checked `LocalInfo::missing()` to fetch the remaining ranges over another connection and reconstructed identical bytes. Observed 66,120 B first-stage protocol response and 2,039,637 B second-stage protocol response. In-memory MemStore remained alive, so **not** a disk/durable-restart claim.

**Follow-up comparison:** pin exact generated Cargo.lock after SHA256 verification, enforce `--locked`, run 10 independent repetitions over selected RTT/loss profiles with identical peer/key/storage policy, and test disk quotas + interrupted large transfer. Compare the result to the existing Staya chunker only after controlling for ACK count and network conditions.

Current acceptance confirms identical dependency closure, single-host QUIC authenticity/integrity negative cases, same-layer UDP instrumentation and one in-memory reconnect/range resume. The remaining controlled A/B needs 10+ repetitions, bounded RTT/loss shaping, same-backend cold/warm measurements, process/disk restart, 2PC WAN/relay tests and authorization/quotas. Only after that may an optional product adapter be considered.

## CI and acceptance

The `.github/workflows/staya-iroh-blobs-lab.yml` uses a read-only GitHub token, pinned toolchain, committed `Cargo.lock` (Git blob `5bff8451c3be8b2c479fd759e6c3c3ecea213ed1`, independently SHA256-verified against initial successful test artifact), exact forks and machine-checkable metadata. `cargo metadata --locked`, `cargo tree --locked`, and `cargo test --locked` are required. The **final exact-head `--locked` CI remains a gate until GitHub reports success**. The temporary single-use workflow that wrote this lockfile has been deleted and must not reach `main`.

Required to promote beyond this lab: reproducible cross-platform builds with pinned deps, malicious read/write tests with disk quotas, unsafe-remote denial and cancellation, real two-PC LAN/WAN tests, direct/relay path attribution, durable staging after restart, independent approved grants, File Gate commit and rollback. Prohibit any implicit migration of canonical S3 V1/V2 or local Akula history.

## Sources

- [Iroh 0.103.1 API and production warning](https://docs.rs/iroh-blobs/0.103.1/iroh_blobs/)
- [Iroh push/access-mask security fix (2026-10-08)](https://www.iroh.computer/blog/iroh-blobs-push-fix)
- [Iroh-blobs request mode implementation](https://github.com/n0-computer/iroh-blobs/blob/main/src/provider/events.rs)
- [Iroh-blobs historical PUSH negative tests](https://github.com/n0-computer/iroh-blobs/blob/main/src/tests.rs)
- [Current Akula Staya framing](https://github.com/Lalalalendia/Akula/blob/feat/staya-roha-transport-01/staya/roha-transport/src/transfer.rs)
- [Previous P2P-DELTA-04 numerical comparison](https://github.com/Lalalalendia/Akula/pull/294)

## P2P-BLOB-05b: verified disk recovery after clean shutdown (2026-10-19)

The patched branch adds the **test-only** `iroh-blobs/fs-store` feature and a pinned `tempfile` fixture. The [pinned Cargo.lock](https://github.com/Lalalalendia/Akula/blob/experiment/staya-iroh-blobs-compare-20261018/staya/roha-transport/Cargo.lock) has Git blob `7d69782079c71fdcfa8be673439d191cb42e12b8`, 101,967 bytes, including `redb 4.4.0` and `reflink-copy 0.1.30`. It is **not** a production Akula dependency.

**Upstream compatibility finding:** the published [v0.103.1 Cargo.toml](https://github.com/n0-computer/iroh-blobs/blob/v0.103.1/Cargo.toml) does not include `tokio/rt-multi-thread` in its `fs-store` feature, although `src/store/fs.rs` calls `tokio::task::block_in_place` and `tokio::runtime::Builder::new_multi_thread`. The lab explicitly enables `rt-multi-thread` on its Tokio dependency. No edits were made to Roha, noq or third-party source.

1. [CI #39160244074](https://github.com/Lalalalendia/Akula/actions/runs/39160244074) green: a file of **2,097,165 B** is partially received on a real Roha QUIC connection into an isolated `FsStore`; 65,536 B are verified. `Store::shutdown()` durably closes the private store, then a new instance reloads the verified bitfield and downloads exactly the missing ranges on a new connection. BLAKE3 and every byte match the sender.
2. [CI #39160477748](https://github.com/Lalalalendia/Akula/actions/runs/39160477748) green: **the receiver is a second operating-system process**, not merely a new Rust object. Parent stages/flushes the verified prefix, launches the same test binary with an ephemeral test-only EndpointId/SecretKey and local address hints; the child opens the same private store, independently authenticates/requests missing ranges and flushes the completed file. The parent subsequently reopens and independently checks all bytes. This is a scoped local CI test, not a WAN or File Gate test. Six regular tests passed with one helper ignored during a normal invocation (the parent invokes it separately).

**Critical limits:** these pass after **graceful** `Store::shutdown()`, not a `SIGKILL` during a write, host power failure or torn disk. Upstream explicitly warns that dropping an `FsStore` can lose recently acknowledged writes; do not interpret QUIC ACK or `local_bytes()` as a durable Akula commit. Prefix metadata and complete blobs reside in a test-only temporary directory, no user's document. There is **no verified quota admission**, global bounded disk usage, durable File Gate handoff or binding to an authorized VaultId/DeviceId. A content-addressed hash alone does not grant access.

**Next gate:** test forced process death before `Store::shutdown()` and confirm either validated resumption or explicit fresh/full fallback (never treat an unchecked partial blob as complete). Enforce file count/bytes/CPU limits before any future product adapter. Then 10+ repetitions on controlled RTT/loss, LAN/WAN/direct/own relay and cross-device grants, before deciding whether to adopt `iroh-blobs`. The stacked draft PR must not merge until **all exact-head required checks** are green and product custody remains separate.
