# DELTA-TEXT-01 — compressed changes inside immutable Akula revisions

**Status:** isolated numerical A/B. No production codec, no V1/V2 wire change, no S3 network call or File Gate apply. Host-custody P0 remains an independent release gate.

## Why test this

Today V2 already finds **new revisions** cheaply, but an AKPKEXP1 pack member still stores the full revision JSON plus full changed blob bytes. A member blob is capped at 32 KiB, the whole pack at 512 KiB. A separate canonical V1 full blob is still uploaded and available under its BLAKE3 key. The conservative pinned V1 checker also consumes the full immutable history.

Therefore a smaller V2 pack does NOT in itself demonstrate lower total S3 upload, storage, GET count or end-to-end sync latency. Large (e.g. 5 MiB) Markdown edits do not even fit the old V2 member limit. A general content-delta protocol needs explicit negotiation and continued V1 availability guarantees.

## A/B without reinventing VCDIFF

Compare three representations including their metadata/verification envelopes:

1. **Full Zstd level 3**: full target, no receiver base required.
2. **One byte splice + Zstd**: unchanged prefix and suffix, deleted span, inserted span; requires exact old bytes. This is a simple control, not a multi-hunk algorithm.
3. **Zstd --patch-from=BASE level 3**: an existing dictionary-based patch algorithm handling scattered edits; also requires the old bytes. This tests the idea without adding a third-party runtime to Akula.

Background: [Zstandard as a patching engine](https://github.com/facebook/zstd/wiki/Zstandard-as-a-patching-engine), [RFC 3284 VCDIFF](https://www.rfc-editor.org/rfc/rfc3284), [Git pack delta](https://git-scm.com/docs/gitformat-pack).

The test-only script is at scripts/benchmarks/sync-text-delta-ab.py; its isolated GitHub Actions job is at .github/workflows/sync-text-delta-ab.yml. It uses **six public repository Markdown documents**, approximately 4–30 KiB, not private notes. Per file it constructs seven deterministic UTF-8 changes: one character; middle sentence; appended paragraph; ten scattered edits; middle deletion; paragraph swap; complete rewrite.

All candidates are round-tripped three times and compared byte-for-byte. Numeric JSONL includes actual compressed *wire bytes including base hash/target hash/length/mode*, median CLI encode/decode wall times, fixture and scenario identifiers. No source content is recorded or printed, and checked-in fixtures are never edited.

### Refusal conditions tested

- Reject missing/wrong old content **before** decompressing the patch; no silent use of newest note as dictionary.
- Check exact output digest and length **after** decoding.
- Reject unsupported format, corrupt/truncated compressed data, forged target hash, invalid splice offsets and oversized results.
- Limit fixture old/new and compressed frames to 2 MiB, bound CLI test output and reject extremely short patch-from bases (Zstd CLI is unsafe for an empty dictionary on some builds).
- Two offline sibling edits share one parent: each reconstructs from that parent; one sibling cannot substitute for the other's base.
- The Python harness uses SHA-256 only for this benchmark. A future Rust implementation must use independently anchored V1 BLAKE3 and exact RevisionIds. Hashes alone do NOT authorize writes or prove remote writer identity.

## Hypothetical selection (not production)

Prefer delta only if the receiver already has the exact independently verified old revision bytes, the representation version is negotiated, and the complete patch message is at least **128 bytes AND 15%** smaller than full compressed text. Otherwise use full Zstd. The 128/15 threshold is a hypothesis to examine, not an approved default.

Any future versioned wire needs explicit base RevisionId and base BLAKE3, target RevisionId/hash/length, bounded compression format, decompression limits and unambiguous fallbacks. Legacy V2 AKPKEXP1 must not accidentally accept new delta bytes under the old raw-member tag.

A missing base or missing client capability chooses the full transfer; mismatching hashes/foreign history are HOLD, not permission to silently rebase edits. No decoder may mutate File Gate, delete local files, confirm a baseline or garbage-collect blobs.

If V1 still must upload every full immutable new blob, patching only the V2 sidecar saves some advisory-read bytes, **not necessarily any total upload**. To cut total S3 PUT bytes would require a separately reviewed content-storage representation with full V1 compatibility/verification. A real S3 two-client A/B must account for old-base cache misses, extra GET/HEAD, cold pin, upload amplification, CPU and RTT, not just encoded patch size.

## Gate for further development

The public repository test corpus is NOT representative user editing behavior. Do not claim average production speedups from these fixtures. If a clear net byte reduction remains with real opt-in histories, implement a separately gated Rust codec with robust negative tests and explicit capability negotiation. The next mandatory Akula milestone remains the host-custody / end-to-end safe File Gate sync path, not this optimization.


## Measured first A/B — run 39133595383 (2026-10-18)

The first real Actions run on PR #270 passed its format/negative tests and all **42** public-repository Markdown scenarios. [Workflow and machine-readable JSONL artifact](https://github.com/Lalalalendia/Akula/actions/runs/39133595383), artifact ID **12138976912**.

| Hypothetical transfer over 42 independent generated edits | Bytes |
| --- | ---: |
| Full text compressed with Zstd-3 | 311,819 |
| One-region splice + Zstd-3, forced for all edits | 122,849 |
| Zstd patch-from, forced for all edits | 86,519 |
| Example 128-byte / 15% adaptive selection | **86,222** |

The example policy chose Zstd patch-from **21** times, splice **15** times and full Zstd **6** times. It reduced message-envelope bytes **72.35%** on *this specific manufactured workload*. In six complete rewrites it selected full Zstd (no claimed saving). The largest improvements were one-character changes (38,943 -> 611 B) and scattered 10 edits (39,140 -> 1,099 B), aggregated across six files.

Across these short CLI-driven scenarios the medians were **1.33 ms** full Zstd encode, **2.65 ms** splice+Zstd encode and **1.35 ms** patch-from encode; corresponding decode medians **1.29 ms**, **1.26 ms** and **1.48 ms**. These include external process startup and are NOT equivalent to an in-process Rust codec benchmark or end-to-end S3 latency.

**Very important denominator:** current V1 still stores an independent full blob for every new content revision. Across the same 42 hypothetical changes that adds about **655,438 raw bytes**, untouched by this experiment. In an artificially simplified scenario with *exactly one* V2 sidecar transfer alongside *exactly one* full V1 upload per change, total moves from 967,257 B to 741,660 B — at most **23.32%** before S3 requests, manifests, old-base retrieval and metadata. Even this is NOT an observed S3 result and would be wrong for many real workflows. Do not report 72.35% as upload saving, sync speed, user storage reduction or energy reduction.

The benchmark data are public product documentation and a test Markdown fixture, not real note edit histories. Before choosing a production algorithm, test actual version pairs (including multi-hour offline edit sequences and documents over 32 KiB), cold/missing base, independent V1 availability, realistic RTT and cloud billable request counts. Keep P0 host-admission priorities ahead of this optimization.
