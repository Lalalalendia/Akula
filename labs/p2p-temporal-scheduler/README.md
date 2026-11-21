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
cargo run --manifest-path labs/p2p-temporal-scheduler/Cargo.toml
```

Fixture: A has a verified chunk at slot 0, bridge R is online at slots 0 and
2, two receivers are online only at slot 2. A -> R at 0, then R -> C/D at 2
meets both deadlines without S3. The one-slot S3 quota allows only one
last-minute on-demand delivery. Synthetic outcome is not a production benchmark.

## Current simplifying assumptions

- Every transfer takes exactly one discrete time slot; new chunks are
  shareable no earlier than the next slot.
- Availability, source chunk inventory, link eligibility, upload/download
  quotas and receiver deadlines are perfectly known (clairvoyant model).
- No file sizes, per-link rates, variable durations, churn, relay cost,
  malicious peers, real content verification or measured bandwidth.
- `share_enabled` controls whether a peer may upload; presence of a chunk is
  never authorization.
- S3 is the authoritative fallback, available to online receivers only,
  with a shared per-slot transfer cap.
- The objective favors timely completion and only weakly penalizes S3
  per route. This is **not** an optimal minimum-cost flow algorithm.
- The replay validator checks link eligibility, online windows, consent,
  capacity, uniqueness and causality; it does not authenticate actual bytes.

## Next gates, before any integration

1. Green exact-head Rustfmt, unit tests, and clippy on Linux and Windows.
2. Randomized small-case tests against an exact scheduling oracle.
3. Variable transfer durations, chunk size and network bandwidth.
4. Reproducible >=100 scenario comparison of completed demands, S3 bytes,
   peer overhead, latency, CPU and forecast error.
5. Integrate only through a default-off advisor after the P2P Beta completes;
   never bypass independent identity/consent, File Gate or signed manifests.

Related Notion specifications:
- https://app.notion.com/p/40332a84beec81c0a1bbd9e5c02c7de0
- https://app.notion.com/p/40132a84beec81d196dee3fcb26ff8e6
