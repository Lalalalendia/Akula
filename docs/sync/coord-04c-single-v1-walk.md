# SYNC-COORD-04c — reuse one verified V1 ancestry walk

Status: **read-only performance candidate**, never authorizes user file writes, baseline advancement, remote GC or automatic sync. Requires #275; the 100-generation A/B fixture from #279 is the acceptance benchmark.

## Observation

The 03b locator discovery entrypoint calls `walk_s3_delta_from_saved_anchor` to prove each missed immutable V1 frame. Its 02e selective-pack verifier subsequently performs the same network walk a second time. In the measured localhost MinIO 100-generation / 101-revision fixture, V2 used 605 GET / 1,210 HEAD requests and 2,502 ms against a full V1 pin's 410 GET / 820 HEAD and 1,329 ms. The two read modes have different old-blob availability assurances.

## Change

- Pass the **single bounded canonical** `S3DeltaWalkPreview` from deterministic locator discovery into the internal V2 selective verifier; no wire changes, manifest rewrite, S3 LIST, checkpoint promotion or public API changes.
- Re-run `inspect_s3_manifest_delta` **in memory** against the exact loaded and profile-scoped SQLite anchor with the same 8/128-frame and shard-reference budgets. Reject altered/partial/non-checkpoint history; do not trust locator hashes on their own.
- Retain the full existing 02e V1 shard-inventory reconstruction, V2 letter and pack/member BLAKE3 checks, revision DAG verification, and final immutable pointer plus opaque ETag stability check against the observed V1 root.
- Keep the independent S3 ancestor walk for the public explicit-letter API (which has no prior discovery proof), and preserve its eight-generation maximum.

## Acceptance

1. Rust formatting, core checks, S3/MinIO, docs and Main PR gate on the exact final head.
2. Existing 20-generation missing-locator HOLD, all siblings/tombstones and unchanged SQLite anchor.
3. #279 100-generation fixture on the optimized reader: same exact 101 RevisionIds and no baseline changes; compare actual GET/HEAD/bytes/time with the fixed 04b baseline. Report raw numbers even if V2 remains slower.
4. No production auto-switch to V1, no claim that cached older blobs have been re-proved available today.

This removes redundant network verification, **not** the cryptographic/causal checks. WAN RTT and 10k-files require separate measurement before adopting a production selector.
