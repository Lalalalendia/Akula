# AK-P2P-TEMPORAL-01: offline P2P scheduler laboratory

This is a **non-shipping, std-only** Rust crate. It has no network, credentials,
S3, Vault, file reads, or production authorization. Do not wire its decisions
into Akula permissions.

It adapts temporal-graph ideas from the Akhintek olympiad problem for a
different objective: serve **unique** receiver/chunk demands before their
deadlines, using cached public content on consenting peers where possible.
An offline time-expanded greedy route planner is compared with S3-only and
on-demand peer-first baselines.

## Run

```sh
cargo test --manifest-path labs/p2p-temporal-scheduler/Cargo.toml
cargo run --manifest-path labs/p2p-temporal-scheduler/Cargo.toml --bin akula-p2p-temporal-scheduler-lab
cargo run --manifest-path labs/p2p-temporal-scheduler/Cargo.toml --bin benchmark
cargo run --manifest-path labs/p2p-temporal-scheduler/Cargo.toml --bin duration_benchmark
```

Fixture: A has a verified chunk at slot 0, bridge R is online at slots 0 and
2, two receivers are online only at slot 2. A -> R at 0, then R -> C/D at 2
meets both deadlines without S3. The one-slot S3 quota allows only one
last-minute on-demand delivery. Synthetic outcome is not a production benchmark.

## Unit-slot baseline: simplifying assumptions

- Every transfer takes exactly one discrete time slot; new chunks are
  shareable no earlier than the next slot.
- Availability, source chunk inventory, link eligibility, upload/download
  quotas and receiver deadlines are perfectly known (clairvoyant model).
- No file sizes, per-link rates, variable durations, churn, relay cost,
  malicious peers, real content verification or measured bandwidth.
- `share_enabled` controls uploads; `prefetch_opt_in` is required
  to stage any chunk that the receiver did not explicitly demand. A user demand
  only authorizes reception of its own exact chunk in this synthetic model.
  Neither boolean is a cryptographic capability or production authorization.
  Merely having a chunk never grants sharing rights.
- S3 is the authoritative fallback, available to online receivers only,
  with a shared per-slot transfer cap.
- The objective favors timely completion and only weakly penalizes S3
  per route. This is **not** an optimal minimum-cost flow algorithm.
- The replay validator checks link eligibility, online windows, consent,
  capacity, uniqueness and causality; it does not authenticate actual bytes.

## Duration-aware stress model (separate `duration.rs`)

The optional offline simulation adds variable **synthetic** chunk sizes (MiB),
per-link transfer rates (MiB per time slot), exclusive occupancy of each link
for the full transfer interval, per-device concurrent upload/download limits,
and bounded aggregate concurrent S3 transfers. The receiver and P2P source
must remain online for **every occupied slot**. Chunks can be relayed only
after successful completion; no same-slot use, preemption or partial resume.

It compares four independently replay-validated strategies:
`S3Only`, `GreedyOnDemand`, `WindowAwareDirect` (forecast-aware,
no multihop prefetch), and `TemporalRoute` (multihop with explicit
`prefetch_opt_in`). Thus, the advanced planner is compared with a
simpler strategy that sees **the same predicted future windows**.

The companion `duration_benchmark` uses 120 deterministic synthetic
networks. It evaluates **exactly the same planned schedule** under
observed online traces in which 10% or 25% of nominally online time slots
independently go offline. There is **no retransmission or replanning** in
these observations, intentionally exposing forecast brittleness. Recorded
S3 byte counts are whole-chunk **attempted/nominal MiB**, not actual billed
S3 egress or fractional failed-transfer bytes.

This is not a real network emulator: peer/device aggregate uplink bitrate,
packet loss, rate fluctuations inside a transfer, segment resume, relay and
battery costs, actual link availability, encryption, manifest validation,
and commercial traffic charges are **not** modeled.

## Next gates, before any integration

1. Keep exact-head Linux and Windows Rustfmt/unit/benchmark checks green.
2. Keep the single-chunk unit-slot exact oracle as a separate scope; duration
   results are **not** certified globally optimal.
3. Compare with recorded, consented network traces, packet loss and
   interrupted/continued transfers; include uncertainty-aware replanning.
4. Add a bounded, explicit cost objective for actual S3 egress, relay,
   peer energy, device uplink quotas and transport overhead.
5. Integrate only through a default-off advisor after the P2P Beta completes;
   never bypass independent identity/consent, File Gate or signed manifests.

Related Notion specifications:
- https://app.notion.com/p/40332a84beec81c0a1bbd9e5c02c7de0
- https://app.notion.com/p/40132a84beec81d196dee3fcb26ff8e6

## Measurement caveat

The `benchmark` binary reports deterministic outcome counts, and noisy
machine-specific calculation timings. A single favorable bridge fixture is
not evidence of a global improvement. Benchmark peers have **perfect knowledge
of future availability**; the on-demand comparator does not act on that
knowledge. The gap is not a controlled estimate of live-world benefit.
A numerical gain of one or two synthetic requests is not enough to justify
a shipping planner.
