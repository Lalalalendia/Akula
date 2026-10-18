# DELTA-TEXT-02 — actual Git Markdown edits and two-client S3 cache semantics

**Status: research-only**. No changes to production V1/V2 envelopes, local File Gate, S3 sync publisher, user files, or confirmed baselines.

## Source evidence

DELTA-TEXT-01 modeled 42 edits of static Markdown. This stage compares **adjacent actual commits touching the same Markdown file** on first-party public Akula Git history. The checked-out repository must have complete Git history (fetch-depth 0). For each revision pair we retrieve exact old/new file bytes with git show; skip deleted files, no-op changes, invalid UTF-8, and blobs larger than 2 MiB. The benchmark records commit prefixes, public path names, SHA-256 test digests, wire sizes and timing, not file content.

The sample includes README.ru.md, performance roadmap, the cross-process File Gate incident report, and native Markdown parser testdata. These are genuine consecutive commits but still **not representative of users' private note edit histories**. Both versions are validated byte-for-byte with the existing DELTA-TEXT-01 full Zstd, splice+Zstd and Zstd patch-from decoders. Full-mode fallback remains explicit.

## Two independently constructed provider clients (test only)

Optional --minio-endpoint accepts **only http://127.0.0.1, http://localhost or http://[::1]**, and creates a fresh randomized disposable bucket. One boto3 S3 client writes verified full old/new blobs and two experimental sidecar messages, the other reads them. No production Akula S3 keyspace or mutable root is touched. After testing the bucket is deleted. No network operation is performed by the comparison without the endpoint switch.

Per actual Git pair, compare these independently verified receiver paths:

- **full baseline:** S3 HEAD + GET of full Zstd message; decode without an old version.
- **warm:** S3 HEAD + GET of selected Zstd patch/splice/full message; exact old bytes already independently verified in the receiver cache.
- **cold:** S3 HEAD + GET of old immutable raw V1-like blob plus S3 HEAD + GET of selected patch, then verify both old SHA and reconstructed target SHA (unless full encoding is selected).
- **old base absent:** receiver does not guess a dictionary; it fetches the full Zstd message. Wrong cached base bytes and wrong target digest must fail.

Important: the old full blob used to test cache miss is a *test-object analogue*, not a real already-verified Akula V1 history anchor. The algorithm MUST NOT use a SHA-256 result as the authority to apply remote content to files. A future production protocol requires independently verified base RevisionId and BLAKE3, bounded decompression and versioned capability negotiation.

MinIO counters are provider HEAD and GET calls plus response body payload bytes, not TCP packets, retries, cold DAG scans, S3 billing, cache validation or real internet latency. Simultaneous full+selected sidecar objects are uploaded only in the temporary test bucket; the cost model explicitly counts the still-required **full new V1 blob** independently. Sending a smaller V2 sidecar cannot eliminate that full V1 upload or existing prepublication proof.

## Decision rule

Only consider a delta if the recipient is already known to hold an independently pinned exact base and the entire message saves >= 128 bytes **and** >=15% compared with full-Zstd. On a cache miss, compare fetching the old base plus delta against sending full. Always preserve the full fallback, conflicts, tombstones, and old reader compatibility.

Do not enable a user-facing codec, schedule a sync agent, switch the V1 blob format or promote local baselines on the strength of this experiment. Existing host custody P0 and the two-client File Gate end-to-end acceptance remain the actual release blockers.

## Measured first run — real Git history and two S3 clients

[GitHub Actions #39135781977](https://github.com/Lalalalendia/Akula/actions/runs/39135781977), artifact ID **12139936077** (17-row JSONL, SHA-256 fixture digests, no Markdown content).

This is **17 actual adjacent Git-version pairs from 5 first-party public Markdown files**. No manually generated one-character changes were added to this corpus. The largest new text was 29,446 bytes and all were within the old V2 32 KiB member limit. All 17 selected Zstd patch-from under the experimental 128 B / 15% threshold; this biased corpus consists of successive documentation edits and is NOT proof that user edits always favor patching.

| Measurement | All 17 transfers |
| --- | ---: |
| Full Zstd-3 envelopes | 139,916 B |
| One-region splice+Zstd-3 envelopes | 46,812 B |
| Zstd patch-from envelopes | 14,171 B |
| Saved V2-only message bytes when old base is present | 89.87% |
| Required independent full new V1 blobs | 364,019 B |
| Hypothetical full V1 + full-Zstd-sidecar sender bytes | 503,935 B |
| Hypothetical full V1 + adaptive-sidecar sender bytes | 378,190 B |
| **Hypothetical sender payload saving including required V1** | **24.95%** |

Actual independent test provider S3 clients on disposable localhost MinIO:

| Verified receiver route | S3 GET | S3 HEAD | Returned object-body payload |
| --- | ---: | ---: | ---: |
| Download full Zstd | 17 | 17 | 139,916 B |
| Cache hit — download only Zstd patch | 17 | 17 | 14,171 B |
| Cache miss — fetch raw old V1-like blob then patch | 34 | 34 | 350,302 B |
| Old base absent — use full compressed content | 17 | 17 | 139,916 B |

**Cache miss interpretation:** the strategy that first fetches a raw old base to apply a tiny patch returned 2.50x more object-body bytes and used 2x GET/HEAD than fetching the complete compressed target. It was worse in **all 17 individual pairs**. A production sender should *not* require extra remote old-base downloads just to enable a patch; it must send full compressed bytes or choose from already-available verified bases. Never speculate that a receiver has a base solely from its manifest ID.

Median local CLI-driven encoding times (per pair): full Zstd 1.270 ms, one-splice Zstd 4.804 ms, Zstd patch-from 1.182 ms; median decoding times 1.136, 1.111 and 1.319 ms respectively. Median localhost S3 path wall time (HEAD+GET) was ~3.38 ms for full vs ~3.38 ms for warm patch; on a cold base path (2 HEAD+2 GET), ~6.65 ms. Single-run medians include process startup and local runner noise, NOT a statistically meaningful latency advantage.

Important constraints:

- The actual loopback test uploads **both competing sidecar representations**, plus full old/new raw blobs, for measurement. The sender numbers in the table are *counterfactual accounting*, not bytes uploaded by that one test execution and not AWS billing.
- No V1 root, cloud CAS journal, source File Gate, trusted BLAKE3 revision anchor or user baseline is changed. Boto3 writer and reader are separately constructed sessions but share temporary MinIO test credentials; this does not establish remote writer identity.
- Production still has to verify old content, changes in the revision DAG, tombstones and offline siblings; HTTP headers, retries, TLS, cold full pinned reads and authentication were outside the probe.
- Dataset selection favors documentation that was edited incrementally and has a short Git history. Need real opt-in sequential user note revisions and larger files before introducing a Rust content codec or revising V1/V2 storage.

**Provisional technical verdict:** reference patching is compelling for a **verified warm base**, while cold-base on-demand download is a losing default. Retain a full-content representation and cache-aware negotiation. The integration/release P0 for File Gate and ordinary Core startup still outranks this optional optimization.
