# Literotica Discovery Lab M4 — offline bounded large-work publication

**Verified locally:** 170 discovered tests; 169 PASS / 1 SKIP (real MinIO not available).
No HTTP requests to Literotica are made. See [M4-ACCEPTANCE.md](M4-ACCEPTANCE.md)
for the code/test matrix, memory measurements, restrictions and next tasks.

- `discovery_lab/streaming_publisher.py`: read a synthetic work one 256 KiB
  string chunk at a time; validate expected chapters/pages and canonical fixture
  URLs; spool `story.md`, chapters, metadata + immutable SHA-256 manifest.
  It is byte-for-byte compatible with the M2 output for identical small inputs.
- `StreamingWorkPublisher`: upload validated staged files via small PUT,
  bounded stream or resumable multipart. Verify each remote object, then commit
  `works/<id>/current.json` using CAS *last*. No full-book RAM buffering.
- `discovery_lab/catalog_index.py`: publish explicitly selected, sorted work
  snapshots as immutable NDJSON shards and a generation manifest, with atomic
  `indexes/current.json` pointer update. This is NOT full-site coverage.
- `discovery_lab/bench_m4.py`: reproducible per-subprocess local memory tests.
  10 MiB and 100 MiB inputs: peak RSS ~95.4 MiB in this environment.
- `tests/test_m4.py`, `tests/test_m4_index.py`: 35 new cases for the two modules.

Quick local verification:

```bash
python3 -m pip install -e '.[test]'
python3 -m unittest discover -s tests -q
python3 -m discovery_lab.bench_m4 --sizes 10 100 --out reports/m4_memory_benchmark.json
```

**Not tested:** real MinIO/AWS/Yandex, a production signed S3 endpoint, 500 MiB
resource limit (timed out), automated Literotica scraping, Akula UI import.

---

# Literotica Discovery Lab M3 — verified offline multipart and direct S3 streaming

**M3 is completed for synthetic/local test harness only.** 135 discovered tests,
134 PASS / 1 SKIP (the genuine MinIO test needs external operator setup).

- `discovery_lab/multipart.py`: `MultipartFileUploader` (8 MiB parts, SQLite
  WAL journal, resume + abort, conditional Complete, streamed SHA readback) and
  `StreamingObjectUploader` (no mandatory local file, bounded nonseekable input).
- `discovery_lab/s3_probe.py --probe-multipart`: **opt-in** real S3/MinIO
  capability check; refuses backends that ignore conditional completion.
- `tests/test_m3.py`: 27 new Boto3 HTTP fixture and resilience tests.
- [M3-ACCEPTANCE.md](M3-ACCEPTANCE.md): results, protocol, known limitations,
  honest MinIO status and operator checklist.

The program never contacts Literotica. The M2 book publisher **still caps an
assembled file at 2 MiB**, although the new standalone S3 transport supports
large files/streams. The local S3 fixture does NOT validate SigV4, and cannot
replace a genuine MinIO test. See M3-ACCEPTANCE.md before deployment.

---

# Literotica Discovery Lab M2 — synthetic, standalone, offline first

**M2 is implemented and exercised on synthetic fixtures only.** The source
website is not contacted; the real MinIO test is optional and was skipped in
this environment. The previous M1 project is retained below for reference.

- `discovery_lab/publisher.py`: assemble synthetic pages -> chapters -> book,
  write portable Markdown/YAML tags plus JSON metadata, content-addressed
  immutable versions and manifest; conditional `current.json` last.
- `discovery_lab/integration.py`: two-pass M1 discovery and verified RAW
  evidence required before a matching synthetic work may be published.
- `discovery_lab/objects.py`: create-only file/S3 objects and `cas_verified`
  for a single current pointer. File CAS is Linux/POSIX fixture-only.
- `discovery_lab/s3_probe.py`: explicit opt-in provider probe now supports
  `--probe-cas` to check `If-Match` and stale rejection against a real S3 server.
- `tests/test_m2.py`: race, failure, completeness, tag, manifest, rollback,
  recovery and signed-SDK HTTP-fixture tests.

**Tested:** 108 discovered tests; 107 PASS, 1 SKIP (real MinIO integration).
`python3 -m unittest discover -s tests -q`.

Quick start (Python 3.11+):

```bash
python3 -m pip install -e '.[test]'
python3 -m unittest discover -s tests -v
python3 -m discovery_lab.cli demo --out ./out-m0
python3 -m discovery_lab.cli rescan-demo --out ./out-m1
python3 -m discovery_lab.cli publish-demo --out ./out-m2
```

For details, file layout, limitations and the optional real MinIO probe, see
[M2-ACCEPTANCE.md](M2-ACCEPTANCE.md), [MINIO-ACCEPTANCE.md](MINIO-ACCEPTANCE.md).
Current artifact limit is **2 MiB per individual file**, no multipart. The
standalone publisher is NOT a real Literotica scraper and accepts only
`https://fixture.invalid` synthetic input. No running Akula or Erida is needed.

---

# Literotica Discovery Lab — M1, offline-safe

**This is a synthetic catalog/discovery experiment, NOT a web scraper.**
There is **no client for Literotica** in this project; all source pages are
synthetic JSON/HTML fixtures under `https://fixture.invalid/`.
This project never downloads creative works. M1 extends M0 with independently
verified rescan passes and a **real Boto3 HTTP exchange against a local
unauthenticated S3-like test fixture**. It has **not been tested on MinIO/AWS/Yandex**.

The Literotica terms page includes restrictions on scraping/copying; an
appropriate authorization and scope review are required before any live
collection: https://www.literotica.com/resources/tos .

## Run the test lab

Python 3.11+; `boto3` is used only by the S3 HTTP tests and the explicit probe.

```bash
python3 -m pip install boto3
python3 -m unittest discover -s tests -v
python3 -m discovery_lab.cli demo --out ./discovery-out
python3 -m discovery_lab.cli rescan-demo --out ./rescan-out
python3 -m discovery_lab.bench --sizes 1000 10000 50000 --out reports/m1_bench.json
```

Expected in the published M1 bundle: **61 PASS, 1 SKIP (optional real MinIO)**.
The S3 protocol fixture starts an HTTP server **only on 127.0.0.1 with a random
local port** and ignores authentication; it does NOT emulate complete S3 or
validate SigV4. The actual boto3 SDK speaks PUT/GET/HEAD and receives 412
errors. No sockets to Literotica or other source sites are opened.

## Changes in M1

- `discovery_lab/rescan.py`: repeat bounded scans of one partition with **two
  consecutive clean identical observations** required for `STABLE_OBSERVED`
  or `RECOVERED_STABLE_OBSERVED`. Identifiers and order must agree.
  Agreement is only over that bounded partition and time window, **not** a
  guarantee of historical or full-site coverage. All earlier pages, snapshots,
  and evidence remain queryable in SQLite.
- `discovery_lab/engine.py`: assesses `DRIFT`, `ORDER_DRIFT`,
  `HOLD_INCOMPLETE` separately; fences completion of expired leases. After
  unknown S3 acknowledgement or simulated crash **recovers RAW directly from
  object storage before another source fetch**; a storage outage defers the job.
- `discovery_lab/html_fixture.py`: strict synthetic embedded JSON page support.
  If a visible fixture and a JSON fixture disagree, the page is quarantined
  as `NEEDS_REVIEW`, not silently committed.
- `discovery_lab/objects.py`: local `FileObjectSink` publishes via hard-linked
  fully written temporary files; S3 sink uses conditional create-only PUT,
  GET readback, SHA-256 and closes HTTP response bodies.
- `tests/s3_http_server.py`: tiny unauthenticated S3-like HTTP fixture with
  persisted object bytes, HTTP 412 preconditions, conditional writes and
  injected lost acknowledgements/corrupted reads. **Not MinIO**.
- `discovery_lab/s3_probe.py`: **explicit opt-in** MinIO/AWS endpoint probe.
  It writes a single uniquely named test object into an existing test bucket,
  checks create-if-absent/readback/rejected overwrite, removes it and checks
  removal. Fails closed if cleanup cannot be confirmed.
- `tests/test_minio_optional.py`: manually enabled integration against an
  operator-provided real MinIO service. **Skipped** in published results.

## Reproducible M1 rescan case

`python3 -m discovery_lab.cli rescan-demo --out ./rescan-out`

1. First pass: four declared, `[1,2]` then `[2,4]` — duplicate `2` and missing
   `3`, status `DRIFT`.
2. Second pass: `[1,2]`, `[3,4]`, status `MATCHED_DECLARED`.
3. Third pass: same four identifiers in same order, status `MATCHED_DECLARED`.

Result: `RECOVERED_STABLE_OBSERVED` with three independent run IDs, all raw
receipts durable, first-pass failure retained and recovery documented.
If two individually complete scans disagree on **identities** or **order**,
they remain `DRIFT` or `ORDER_DRIFT` and cannot publish stability.

## Actual benchmark (M1, local synthetic fixture only)

150 items/page; process measured on Python 3.13, local filesystem and SQLite.

| Entries | Pages | Elapsed | Throughput items/s | p95 page | Disk incl. SQLite |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1,000 | 7 | 0.049 s | 20,369 | 8.74 ms | 0.56 MiB |
| 10,000 | 67 | 0.564 s | 17,734 | 13.97 ms | 5.03 MiB |
| 50,000 | 334 | 2.768 s | 18,066 | 11.50 ms | 25.24 MiB |

Process-wide high-water RSS was 107.2 MiB after all three runs, not a per-run
steady-state measurement. Benchmarks include zero real S3/MinIO or website
network round trips. Network throughput/cost predictions are **not supported**.

## Optional real MinIO integration

See [`MINIO-ACCEPTANCE.md`](MINIO-ACCEPTANCE.md). Requires a separate private
bucket, explicit credentials and endpoint; test does not run by default.
The sandbox that generated this archive **had no MinIO/Docker installation
and could not resolve the MinIO download host**. Do not treat local S3-like
fixture success as provider acceptance.

## File index

- `discovery_lab/model.py` and `html_fixture.py`: synthetic typed JSON/HTML.
- `discovery_lab/engine.py`: SQLite frontier, claims, leases, snapshots, edges,
  report, diff, cross-pass assessment, retry and recovery.
- `discovery_lab/rescan.py`: bounded multirun reconciliation.
- `discovery_lab/objects.py`: local immutable and opt-in S3 object sinks.
- `discovery_lab/s3_probe.py`: controlled live private-bucket capability test.
- `discovery_lab/cli.py`: demo and rescan-demo, never accepts real source URL.
- `tests/`: legacy M0, new M1, local HTTP S3, opt-in MinIO.
- `reports/`: actual test output, synthetic rescan and local benchmark.

## Not implemented

- Live Literotica site fetch/parser, robots/permission acceptance, work text.
- Genuine MinIO/S3 provider testing in this generation environment.
- Multipart, CAS manifest publication across multiple writers, signed tests,
  IAM/security/performance of cloud providers, production fetch admission.

## Notion

- Discovery design: https://app.notion.com/p/40232a84beec815a9a29ecacf3c2b6d1
- M0 implementation report: https://app.notion.com/p/40332a84beec8194ab90c4aaaf1cd9c2

---

# M5 — one executable ingest chain

`discovery_lab/unified_pipeline.py` + `unified_cli.py` integrate the existing M4
SQLite discovery, verified RAW snapshots, a separately restartable HTML
parser, the streaming Markdown/JSON assembler and immutable file/S3 publisher.
The result is an explicit `verified-source-handoff-v1` for Erida and the
Literotica archive, **not** an automatic import or live-site scraper.

```sh
python -m discovery_lab.unified_cli demo --state ./out-m5
python -m discovery_lab.unified_cli demo --state ./out-m5
```

See [M5-PIPELINE.md](M5-PIPELINE.md) for stage contracts, acceptance scope,
source permissions and integration points.
