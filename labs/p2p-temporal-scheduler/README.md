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
cargo run --manifest-path labs/p2p-temporal-scheduler/Cargo.toml --bin recovery_benchmark
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

## Bounded verified-segment recovery experiment (separate \`recovery.rs\`)

The opt-in **offline model** represents an immutable, public chunk as individually
verifiable 1-MiB segments. A recipient can preserve a verified prefix after
any observed offline slot, select a different permitted source, and request
only the missing suffix. A peer may upload a chunk **only when the full chunk
was verified at the beginning of that slot**. Same-slot forwarding and sharing
incomplete copies are prohibited. The underlying software does **not** verify
Merkle/Bao proofs or perform real disk/network operations; segment verification
is an assumption of the synthetic simulator, not security evidence.

The scheduler reads the observed availability **only at the present slot**.
It uses the common forecast from the network input for any future lookahead.
All policies get the same observed trace, deadlines, source inventories,
consent and per-slot link/upload/download limits. Comparison modes:

- \`S3Only\`: incremental S3 delivery with resume.
- \`PeerFirst\`: incremental P2P peer-first, then S3 fallback.
- \`ForecastDirect\`: at every slot, select a direct source by predicted
  completion time with a preference for P2P when tied.
- \`AdaptiveRelay\`: same direct policy plus optional P2P-only prefetch to an
  explicitly consenting and sharing relay when the recipient has a later
  predicted contact window.

The \`recovery_benchmark\` repeats these policies over **120 fixed-seed
synthetic networks**, each under forecast-perfect, 10%-missing and 25%-missing
availability conditions. It also reports the old \`duration::TemporalRoute\`
with a fixed, non-resumable plan as a **separate reference**: those policies
are not identical except for resume, so any observed improvement is *not*
a causal estimate of resume alone. Newly received MiB are counted as verified
without transmission-header/proof overhead. S3 volumes represent the number of
simulated successfully transferred MiB, **not billed egress**. No transport
deadlines, bandwidth sharing beyond configured quotas, packet loss within
a slot, segment-proof size, auth grants, power use, storage wear or privacy
impact is measured.

**Important:** Permission to share (\`share_enabled\`) is distinct from
permission to receive unsolicited content (\`prefetch_opt_in\`). Both are
test-only booleans and provide no Akula authorization. Only existing trusted
Akula enrollment, immutable-root grants, pinned access and File Gate checks
could ever admit actual bytes to transfer.

## Controlled A/B: partial-chunk retention only

The updated `recovery_benchmark` runs the **same** `PeerFirst`,
`ForecastDirect`, and `AdaptiveRelay` reactive selection policies against
identical predicted/observed online traces with one switched factor:

- `PartialMode::KeepVerified`: retain independently verified partial MiB
  across interruptions and source changes.
- `PartialMode::RestartInterrupted`: discard incomplete progress after a
  skipped slot or source change, but **retain fully verified chunks**.

The no-resume control accounts for already transferred but discarded MiB in
`wasted_mib`. This is not billed S3 or transport overhead. Both variants
obey the same consent, source-ownership, upload/download quotas and deadlines.
Independent replay verifies the mode-specific progress and the transferred
segment sizes. Three focused regressions check a 4/5-MiB P2P-to-S3 handoff,
a gap with the same sender, and an uninterrupted transfer.

Observed Linux CI (120 seeded networks, 1,250 receiver/chunk requests):

| Unexpected offline slots | ForecastDirect: restart | ForecastDirect: keep | Extra completed with keep |
| --- | ---: | ---: | ---: |
| 0% | 964 | 1,032 | 68 |
| 10% | 895 | 990 | 95 |
| 25% | 819 | 941 | 122 |

At 25% observed outages, the keep mode transmitted **2,407 MiB S3 +
1,886 MiB P2P**, compared with **2,894 MiB S3 + 2,246 MiB P2P**
in the restart arm. That is 487 fewer synthetic MiB from S3 and
360 fewer synthetic MiB peer-to-peer. The restart arm discarded 2,427
already transferred MiB. Keep completed strictly more requests in 62
networks and tied in 58; restart had no network-level wins.

Interpretation: the controlled **retention rule** is valuable in this model;
this does **not** measure production throughput or guarantee that the
difference generalizes to real segment sizes, network conditions or
file-integrity proofs. Even at 0% surprise outages, forecasted on/off
windows and changed senders can cause interruptions. Replanning policies
may choose different sources **because** the retained progress differs,
which is part of the retention intervention.

## Next gates, before any integration

1. Keep exact-head Linux and Windows Rustfmt/unit/benchmark checks green.
2. Keep the single-chunk unit-slot exact oracle as a separate scope; duration
   results are **not** certified globally optimal.
3. Recovery has passed synthetic interruption scenarios; next compare with
   recorded **consented** network traces and actual transport resume/proof checks.
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
