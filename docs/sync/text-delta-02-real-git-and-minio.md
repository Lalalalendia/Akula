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
