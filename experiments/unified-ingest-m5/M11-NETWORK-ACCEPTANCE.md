# M11: true network failure acceptance for the shared ingest pipeline

This experiment **does not scrape websites, stage anything in Erida, or deploy**.
It relies on disposable loopback MinIO with versioning and no production credentials.
It tests the already existing M5–M10 pipeline and its M3 multipart journal.

## Observable scenarios

1. Real TCP disconnect **after** MinIO accepted an immutable PutObject but **before** the client received its success response. A full, SHA-256-checked GET settles the ambiguity. Exactly one S3 object version is permitted.
2. Connection severed **before** the object reaches MinIO. The result must be `ObjectWriteUncertain`. An explicit later retry must produce exactly one object version.
3. Server starts a GET and its TCP stream is truncated. The client rejects incomplete readback. Reopening the connection yields the intact object.
4. Conditional CAS on `current.json` commits while the acknowledgement is lost. The existing pointer is verified byte-for-byte. Exactly one version is permitted.
5. The first multipart UploadPart commits but acknowledgement is lost. The on-disk SQLite journal persists. MinIO is **actually stopped and restarted** (same container and `/data`) before a new `MultipartFileUploader` reopens the journal, checks remote parts and finishes the file. The resulting S3 file has the original length/SHA-256 and one committed object version.
6. CompleteMultipartUpload commits but acknowledgement is lost; a full streaming GET proves completion. No duplicate object versions.
7. A full ingest service job is claimed while MinIO is **really stopped**. The job goes to `NEEDS_REVIEW`, and **operator-controlled** retry after restoration completes three approved fixture pages and a verified immutable work/current manifest. Neither source crawling nor Erida confirmation is invoked.

Test markers: `M11_ALLOW_CHAOS=local-disposable-minio` and `M11_MINIO_CONTAINER=ingest-m11-minio`. The Python fault proxy only binds `127.0.0.1` and forwards signed requests without storing credentials. Every fault is armed for one matching call only. Fail if the fault was not actually triggered.

## Commands

Locally (no Docker required):

```sh
cd experiments/unified-ingest-m5
python -m unittest discover -s tests -p 'test_m11_fault_proxy.py' -v
python -m unittest discover -s tests -p 'test_m10_faults.py' -v
```

Disposable CI with MinIO is defined by `.github/workflows/exp-unified-ingest-m11.yml`, and uploads `m11-network-results.json` even when the test fails. MinIO restarts occur **only** inside the dedicated M11 container.

## Non-goals and remaining limits

- No claims about multi-server cluster leases, external network latency or public S3 providers.
- Recovered MPU requires a **replayable, approved local file**, not a once-only HTTP response. Use the M4 disk spool if a large object must be restart-safe.
- StreamingObjectUploader remains one-shot and must abort on failure or rely on bucket lifecycle cleanup. An interrupted streaming source cannot be reconstructed automatically without a saved copy.
- Unexpected 403/429/503 from a provider, automatic retry budgets, KMS/SSE encryption and certificate validation require separately authorised provider-specific tests.
- The MinIO image is an unverified third-party mirror for ephemeral CI only. Production requires provider approval and immutable image digest/signature verification.
- Actual source adapter for real Literotica content and commercial/legal permission checks remain outside this acceptance.

