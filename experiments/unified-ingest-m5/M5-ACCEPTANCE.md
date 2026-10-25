# M5 acceptance — unified pipeline, 25 October 2026

## Delivered

A runnable, **offline-first** integration of existing M4 discovery,
immutable RAW, parser, streaming work assembly, S3-compatible storage,
manifest CAS and an explicit, read-only handoff for Erida/Literotica.
No live Literotica pages, no Gemini calls, no book publication, no Erida
source job mutations, no Akula core changes.

Commands used:

```sh
python -m unittest discover -s tests -v
python -m discovery_lab.unified_cli demo --state /tmp/unified-ingest-m5-demo
python -m discovery_lab.unified_cli demo --state /tmp/unified-ingest-m5-demo
```

## Acceptance observations

- **189 tests discovered, 188 passed, 1 skipped** (the existing optional
  real-MinIO test is explicitly disabled). This combines 170 original M0-M4
  tests and 19 new M5 tests.
- **Pipeline CLI demo**: M4 catalog discovery twice, stable identities and
  order (`STABLE_OBSERVED`), three approved source HTML pages, RAW readback,
  parser, 2 chapters, M4 immutable Markdown+JSON and pointer, read-only
  handoff. First run `PUBLISHED`; exact repeat `UNCHANGED`.
- **Fetch/parse independent**: RAW can be fetched and stored before parsing;
  the fixture HTML can be deleted before the parse phase without causing
  new source retrieval. This enables parser-version reprocessing from RAW.
- **Durability**: crash injected after verified RAW write and before SQLite
  commit, lease expired, rerun completed. No duplicate RAW object appeared.
- **Fail closed**: wrong fixture schema, bad UTF-8, wrong ordinal, corrupted
  RAW bytes, changed approved manifest, path traversal, missing text prevent
  publication.
- **S3-compatible HTTP fixture**: M5 integration tests exercise boto3 SDK
  against the existing M4 in-process HTTP S3 fixture, including idempotence,
  corruption rejection and recoverable RAW acknowledgement. This is **not**
  MinIO, AWS, Yandex or production protocol acceptance.
- **Rust handoff**: M5 accepts already verified RAW keys and SHA-256 receipts
  (for example from the bounded Rust fetcher), without reading local source
  HTML. This test uses a synthetic immutable object; Rust process is not
  launched by M5 test code.

## Remaining gates before actual source sites

1. Site-specific permitted discovery and page HTML parsers, verified against
   rights-cleared saved snapshots; no silent completeness assumptions.
2. Consistent source/actor identity and quotas in Erida; no automatic call to
   translate/publish or its live workers.
3. Literotica parser & catalog/index adapter mapping author, work, series,
   chapter and page identities, preserving many-to-many series membership.
4. Production-ready parser version migrations and revisioned SQLite manifest
   records; the current demonstration treats the selected plan as immutable.
5. Full M5 end-to-end true MinIO/S3 test and credential management; independent
   validation of conditional requests, multipart and cloud storage failures.
6. Domain/robots/ToS and site rate limits; no live fetcher is implemented by M5.

The earlier GitHub PR #726 demonstrates a bounded **Rust HTTP -> MinIO**
transport at 20/1000/10000 synthetic pages. This M5 package adds the
orchestration, parsing, publishing and handoff to the preexisting M4 lab.
Their independent successful tests are **not** represented as a deployed,
combined production system.
