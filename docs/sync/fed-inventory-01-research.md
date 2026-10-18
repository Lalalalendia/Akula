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
