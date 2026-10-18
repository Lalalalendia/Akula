# FED-INVENTORY-01 — restricted read-only reconciliation of Akula peer inventories

**Status:** isolated experimental Rust crates + automated verification. NOT Akula product, no existing Sync Core/DAG/File Gate/S3 writer changes, no real network traffic and no third-party production dependencies. No user files are used.

## Goal and prior work
Akula already has typed VaultId/RevisionId, an immutable revision DAG with missing-parent safeguards, a SQLite journal, a safety-first planner, and a **read-only S3 manifest delta walker** for an authenticated linear S3 lineage. That is not the same problem as comparing the partially overlapping, independently collected metadata sets of two federated peers.

[iroh-docs v0.101.0](https://github.com/n0-computer/iroh-docs/blob/v0.101.0/src/ranger.rs) uses **range-based set reconciliation**; it does not replace Akula's notion of authorization, revision ancestry, tombstones, or File Gate. The lab here intentionally **does not claim exact iroh-docs protocol compatibility**. Instead it evaluates another deterministic, prefix-partitioned BLAKE3 digest approach and checks its candidate differences against an independent exact full-list baseline.

## Exactly what the prototype does
- Input: two authorized *test fixtures* with the same scoped 16-byte simulated Vault identity; each contains distinct 32-byte RevisionIds and 32-byte content digests. They are **not actual signed Vault grants**.
- Sort and reject duplicate RevisionIds; reject mismatched Vault and denied simulated access before comparing any inventories.
- Compute a domain-separated BLAKE3 digest for each canonical prefix range, including item count, IDs and exact content digests. Equal digests stop traversal; unequal digests are split on the next high-order bit until <=32 records or exhausted key bits. Then exchange exact pairs for that range.
- Fail if the explicit node-visit budget is exhausted, rather than returning and publishing a partial list.
- Output is *only candidates*: missing IDs on either side and same-ID/different-hash collisions; no deletion, merge, local writes, S3, network, or File Gate actions. Cryptographic digests do not prove the claimed party's authorization or the existence of actual signed records.
- A separate, simple full-list comparison uses ordered maps instead of the prefix-tree code, to catch comparison errors.

**The byte counter is a transparent lower-level protocol estimate:** 80 bytes per two 32-byte digests and two 8-byte counts; 16 bytes leaf count header plus 64 bytes per explicit ID/digest pair. It is NOT QUIC/UDP/IP wire traffic, does not include framing/signatures/retries, and should never be compared to actual throughput of PR #301.

## Cases and expected invariants
1. Identical 10,000-item lists: one comparison round, zero missing entries.
2. One missing entry: exact matching list of candidates and fewer transmitted bytes than full listing.
3. 100 missing and 50 extra; half missing; fully disjoint; same ID with conflicting digest; shuffled order and duplicate IDs.
4. A denied access fixture, wrong Vault, or visit limit: strict error, no returned partial candidate set.
5. All successful results equal independent full-list comparison, including collision records. Missing entry is **never** proof that an item should be deleted from another node.

## Independently reproducible suspected bug in iroh-docs
The separate crate at [experiments/staya-authorheads-01](../../experiments/staya-authorheads-01) depends on the **actual published iroh-docs 0.101.0** with our pinned Roha/noq forks. It checks:
- GREEN control: distinct timestamps for two AuthorIds round-trip.
- **Expected RED regression:** two different authors sharing the same timestamp must survive encode(None)/decode, but in upstream [src/heads.rs](https://github.com/n0-computer/iroh-docs/blob/v0.101.0/src/heads.rs) encode first builds BTreeMap<Timestamp, AuthorId> and one key overwrites the other. The CI step treats *only this specific expected assertion* as confirmation; a compilation or dependency failure does not count as reproduction.

This candidate is limited to the **AuthorHeads serialization summary**. It is not proof that the actual signed records disappear or that full range-based synchronization fails. No upstream PR is intended; contribution is paused.

## Commands used by the isolated CI
```sh
cargo generate-lockfile --manifest-path experiments/staya-fed-inventory-01/Cargo.toml
cargo test --locked --manifest-path experiments/staya-fed-inventory-01/Cargo.toml --all-targets
cargo run --locked --manifest-path experiments/staya-fed-inventory-01/Cargo.toml --release

cargo generate-lockfile --manifest-path experiments/staya-authorheads-01/Cargo.toml
cargo test --locked --manifest-path experiments/staya-authorheads-01/Cargo.toml --test upstream_collision distinct_timestamps_are_preserved_control_green -- --exact
cargo test --locked --manifest-path experiments/staya-authorheads-01/Cargo.toml --test upstream_collision equal_timestamps_for_two_authors_must_both_survive_expected_red -- --exact
```

The last command is expected to exit non-zero; the dedicated CI step examines its output. Lockfiles are **generated inside the job** pending this first reproduction and uploaded as artifacts; they are not product dependencies. The workflow uses checkout without persistent credentials and read-only token permissions.

## What a successful lab does NOT establish
- No proof of actual network speed, 3 physical servers, direct LAN/relay/WAN routes, multipath, denial-of-service resilience or large-content lifetime.
- No production grant/ACL, proof of ownership or server signatures, independent provider-truth validation, vault-private metadata encryption, revocation or durable File Gate handoff.
- Prefix hash comparison can be efficient with sparse changes but is not a Byzantine-validating protocol; a dishonest peer may lie. Signed canonical Akula revisions must be validated separately.
- This lab **does not replace** [Akula Sync Core](https://github.com/Lalalalendia/Akula/blob/main/aquilum-app/core/src/sync/dag.rs) or [PR #301 blob laboratory](https://github.com/Lalalalendia/Akula/pull/301).

**Next acceptance:** read the exact CI run and amend this document with results; only then consider an authorized, authenticated read-only candidate exchange between three peers and FED-GOSSIP-02.


## Verified GitHub Actions results (19.10.2026)

[CI #39162935229](https://github.com/Lalalalendia/Akula/actions/runs/39162935229), exact head 4091dc71a5cf5dbf18ac8b089f7827a550a12cac: **two successful jobs**.

- Inventory: **12/12 real Rust tests passed**, including 64 deterministic mixed inventory comparisons, identical/prefix-edge/empty inventories, ID/hash collisions, wrong Vault, revoked simulated grant and late work-budget exhaustion; every successful output matched the independent full-list implementation.
- Upstream iroh-docs 0.101.0: same-timestamp AuthorHeads intentionally RED with **actual 1, expected 2**, exact assertion STAYA_EXPECTED_HEADS_COLLISION_LEN2; different-timestamp control GREEN. The wrapper marks this expected, verified RED as a successful research result.
- [First CI #39162511816](https://github.com/Lalalalendia/Akula/actions/runs/39162511816) passed initial inventory 8/8, but upstream failed to compile because crate iroh-base was duplicated from crates.io and our git fork. Patched iroh-base to own Roha on the *experiment-only* crate; [retest #39162776452](https://github.com/Lalalalendia/Akula/actions/runs/39162776452) then confirmed real upstream RED. No claims of defect in upstream when used with its own compatible dependency graph.

| Scenario | Prefix nodes | Leaves | Estimated protocol B | Full-list B |
|:--|--:|--:|--:|--:|
| Identical 10,000 | 1 | 0 | 80 | 1,280,016 |
| Missing one | 17 | 1 | 5,152 | 1,279,952 |
| Missing 100 | 509 | 88 | 283,920 | 1,273,616 |
| Missing 100, extra 50 | 645 | 129 | 403,872 | 1,276,816 |
| Missing half | 951 | 476 | 1,043,696 | 960,016 |
| Disjoint | 1,029 | 515 | 1,370,560 | 1,280,016 |

**Caveats:** modeled protocol fields, not measured QUIC or UDP bytes. Full-list fallback is better when differences are large, but must be selected before excessive partial exchange; no tested adaptive network mechanism yet. No ACL signatures, durable file commits, 2+ physical servers, relay, or WAN evidence. The AuthorHeads defect concerns a serialized summary, not proven loss of actual signed records. Contribution to upstream remains on pause.
