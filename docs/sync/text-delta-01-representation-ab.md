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
