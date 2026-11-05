# Rust Spider vs Python Scrapy -> real MinIO S3

Self-contained reproducible **experiment**, not production code. It compares **Scrapy 2.19 + boto3** to **Spider 2.53.9 + AWS Rust SDK for S3**. It downloads *exactly the same generated local HTML site*, stores one S3 object per page, checks SHA-256 of every downloaded S3 body, records sampled peak resident memory (RSS), wall/CPU time, and crashes each client **after a successful server PUT** before any local acknowledgement is recorded. On recovery it requires exactly one S3 object **version** per page; this is stronger than counting keys alone.

## Run on a Linux machine with Docker, Python and Rust

```
python3 -m pip install -r requirements.txt
python3 -m unittest discover -s tests -v
docker run -d --name rust-spider-minio -p 127.0.0.1:9000:9000 \
  -e MINIO_ROOT_USER=trial-access -e MINIO_ROOT_PASSWORD=trial-secret-2026 \
  dappros/minio:RELEASE.2025-09-07T16-13-09Z server /data
export S3_ENDPOINT=http://127.0.0.1:9000
export AWS_ACCESS_KEY_ID=trial-access AWS_SECRET_ACCESS_KEY=trial-secret-2026
cargo build --release -j 2
python3 run_benchmark.py wait
python3 run_benchmark.py run --rounds 3
```

The GitHub Actions workflow `exp-rust-spider-s3.yml` also starts a real MinIO container and uploads JSON + per-worker logs as artifacts. The MinIO server runs solely inside GitHub Actions; no public website, cloud credentials, user data or third-party S3 service is accessed. The synthetic fixture and benchmark application bind to 127.0.0.1.

## What the benchmark does not establish

- It is a **single-node, local-loopback S3-compatible server**, not public AWS S3; remote network latency, bills, IAM, multipart, TLS and real-world target sites are out of scope.
- The crawler and S3 client libraries have different scheduling, retry and parsing internals. The outcome is **end-to-end for this user scenario**, not a pure language-speed comparison.
- It crashes client processes after *an acknowledged PUT*; it does not simulate packet loss halfway through a multipart upload, nor does it guarantee exactly-once delivery for arbitrary remote S3 providers.
- No unauthorised scraping: only a synthetic local website is crawled.
- The clean stage alternates order for 3 rounds. The Python startup cost is included, Rust compilation excluded, and RSS is sampled every 20ms. This is indicative, not a statistical performance claim.
- If the Rust build fails in CI, do not report speed measurements; only record compilation blocker and logs.

## Structure

- `fixtures.py`: deterministic local HTML site and independently checked SHA-256 manifest.
- `python_crawler.py`: Scrapy crawler, async item pipeline, signed boto3 S3 uploads.
- `src/main.rs`: Spider crate + Rust AWS SDK signed S3 uploads.
- `run_benchmark.py`: MinIO preparation, object versioning, 3x repeated comparison, crash/restart, full body verification, stats, JSON artifacts.
- `tests/`: fixture assertions, no external dependencies beyond `unittest`.

**Never place real credentials in the repo.** The credentials above are disposable values scoped to the transient MinIO container.

## Diagnostics and fairness
Both clients use ListObjectsV2 and HeadObject for existing S3 objects and a maximum of four parallel uploads. Scrapy fails if scraped count differs from expected. The Rust broadcast subscription is sized to all 160 synthetic pages plus slack, and explicitly errors on Lagged. This is a benchmark workaround; memory can scale with fixture size and should not be represented as production-grade backpressure. `results.json` records failure type and message. A third-party MinIO Docker image is limited to this isolated CI test.
