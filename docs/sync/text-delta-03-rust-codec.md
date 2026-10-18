# DELTA-TEXT-03 — versioned, bounded Rust text representation

**Scope: internal, opt-in Rust Core feature \`sync-text-delta\` only.** This is not a S3 sync sender, not an AKPKEXP1 V2 pack member, not a new V1 blob format, not an installed native application feature and not a File Gate authorization. It is an inert transport **representation** that can be independently examined and discarded.

## Why this step

Research PR #270 and PR #276 showed that existing Zstd reference patching offers large byte savings when the receiver already has an exact old version, while fetching a missing full base only to decode a delta is substantially worse than sending a complete compressed representation. Neither experiment authorized replacing the legacy immutable V1/V2 wire formats.

This feature proves a **Rust-level bounded envelope and hostile input decoder** without connecting it to the live writer or receiver. It uses the maintained \`zstd\` Rust crate (optional and only compiled under \`--features sync-text-delta\`), BLAKE3 and existing typed \`RevisionId\` values. No additional application service is started. Existing \`sync-s3\` does **not** enable the feature.

## Version 1 experimental wire

Distinct fixed magic \`AKTDLT01\`, schema 1, mode 0/1/2, 2-byte zero-reserved, optional base UUID (16), target UUID (16), optional base BLAKE3 (32), target BLAKE3 (32), old length (u32 LE), target length (u32 LE), frame length (u32 LE), then one whole zstd-compressed frame. All 120 header bytes and the frame length must match exactly: no unknown fields, trailing bytes, alternate schemas, nonzero reserved, nil target UUID or mixed full/delta base fields.

- **0: FullZstd** — full target compressed at level 3, no base required or represented.
- **1: SpliceZstd** — one prefix/delete/insert/suffix byte splice compressed with Zstd-3. Even though offsets use raw byte counts, the final UTF-8 target is independently validated.
- **2: DictionaryZstd** — the previous complete text is the *raw Zstd dictionary* for level 3; decoder needs that exact verified base. This Rust dictionary mode is **not byte-compatible with the Python/CLI \`zstd --patch-from\` result**. Its compression efficiency must be measured independently; we make no transfer-% claim based on the earlier experiments.

For now, old and target bodies are restricted to <= **2 MiB** and compressed frames to <= **2 MiB + 64 KiB**. The bounded decoder refuses malformed splices, target lengths, broken Zstd payloads, unsupported mode/version, bad UTF-8, wrong base UUID/digest, missing base, wrong target UUID/digest, truncation and padding. It never silently reuses the newest available revision as the reference or merges concurrent siblings. Full Zstd is always an available base-free representation; delta is selected only if it saves >= **128 bytes and 15%** of the complete full-Zstd envelope.

## Trust and compatibility boundary

The \`encode\` and \`decode\` functions accept only raw in-memory buffers plus typed IDs. The caller must independently prove that the base is pinned to an authenticated current V1 revision DAG, its source bytes and BLAKE3; a content hash is **not** remote writer identity or protected host File Gate admission. The codec cannot change any S3 object, local file, protected-original evidence, baseline or history.

Old \`AKPKEXP1\` packs always contain full raw blob bytes and must strictly reject this magic. The test suite in \`s3_transport/pack_probe.rs\` explicitly passes a newly encoded text envelope to the legacy verifier and proves it is rejected even with matching outer content-addressed BLAKE3. No new reader capability, fallback semantics or support for applying patches is negotiated here.

**Potential important incompatibility:** dictionary-Zstd does not achieve the exact compressed patch format used in PR #276; judge it using the actual Rust ratio, not prior CLI ratios.

## Test acceptance

- \`cargo check --locked -p aquilum-core --no-default-features\` leaves all cloud and codec features disabled.
- \`cargo test --locked -p aquilum-core --no-default-features --features sync-text-delta text_delta_codec::tests\`: byte-exact Unicode/empty/budget-bound round trips, full fallback, independent offline siblings, missing/wrong base, changed UUID, bad mode/schema/reserved flags, corrupted target hash, malformed frames/splices and forged length.
- \`cargo test --locked -p aquilum-core --features sync-s3,sync-text-delta new_text_delta_is_not_a_legacy_pack\`: new content bytes cannot become old-format V2 pack members.
- Full standard Rust/Application and docs CI must remain green. No CLI or Python benchmark is used at runtime.

If these gates pass, record the ratio of **this Rust dictionary mode** on the 17 real adjacent Git pairs from #276. Until then it would be false to report its efficacy as 89.87% just because a different Zstd CLI patch codec showed that result. Only consider a separately versioned, negotiated content transfer format after host-custody P0, an independently verified base cache and an end-to-end S3 receiver test. No legacy V1 full blob elimination is claimed.
