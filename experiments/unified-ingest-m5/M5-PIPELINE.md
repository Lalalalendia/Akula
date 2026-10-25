# M5 — Unified source ingestion pipeline (fixture-verified core)

## What is built

One executable process now **connects** the M4 discovery/RAW/streaming-publisher
pieces with a durable per-page fetch/parse queue and an export-only handoff.
It **does not** start a crawler against Literotica or install anything in Erida.

```text
approved catalog / discovery adapter
         |
    selected-work-v1 (immutable list of pages + explicit expected counts)
         |
    SQLite PageLedger (READY -> FETCHING -> RAW -> PARSING -> PARSED)
         |                              |
    source fixture or already       RAW snapshots in immutable S3
    uploaded Rust transport object        |
         +------------------------------> parser adapter (versioned)
                                         |
                                   immutable parsed-page JSON
                                         |
                                   M4 streaming assembler
                                         |
                                   M4 verified S3 publisher
                                         |
                               immutable revision + manifest
                                         |
                                   current.json CAS (last)
                                         |
                               verified-source-handoff-v1
                                    /              \
                    Erida stage manually     Literotica index manually
```

### Reused, not replaced

* M4 `DiscoveryStore`, `DiscoveryEngine`: synthetic catalog coverage and stable passes;
  no second general-purpose discovery engine is invented.
* M4 `FileObjectSink` or `S3ObjectSink`: create-only RAW / parsed objects with
  readback and SHA-256.
* M4 `StreamingWorkPublisher`: spool to bounded disk, page/chapter completeness,
  immutable Markdown and metadata, conditional `current.json`, multipart for
  larger artefacts where supported.
* `PageLedger`: new **bridge** between generic approved page inventory and M4
  publisher; SQLite WAL, leased stages and atomically persisted small receipts.
  Bodies do **not** enter the SQL queue.
* Existing Rust experiment in Akula PR #726 writes RAW snapshots to S3. Its
  deterministic object keys and SHA-256 values can be placed in a
  `selected-work-v1` manifest with `raw_key` + `raw_sha256`. This is an explicit
  handoff, not automatic cross-repository execution.

## Quick start (no external HTTP)

Install Python 3.11+ and optional dependencies for S3 tests:

```sh
python -m pip install -e '.[test]'
python -m unittest discover -s tests -q
python -m discovery_lab.unified_cli demo --state /tmp/literotica-m5
python -m discovery_lab.unified_cli demo --state /tmp/literotica-m5
```

The first command runs **two catalog discovery passes** with M4, checks that
work identities and order agree, then reads three local synthetic chapter pages,
saves immutable RAW, parses HTML, validates 2 chapters/3 pages, publishes
Markdown+JSON with the existing M4 publisher and writes a read-only handoff.
The repeated invocation returns `UNCHANGED` for the same published revision.

For a selected manifest of already-approved local HTML pages:

```sh
python -m discovery_lab.unified_cli run \
  --plan examples/approved-work/approved-plan.json \
  --fixtures examples/approved-work --state /tmp/approved-book-m5
python -m discovery_lab.unified_cli status \
  --plan examples/approved-work/approved-plan.json --state /tmp/approved-book-m5
```

To prove parsing does not require refetching source pages:

```sh
python -m discovery_lab.unified_cli run --plan examples/approved-work/approved-plan.json \
  --state /tmp/another-m5 --stop-after fetch
# The local fixture HTML files may now become unavailable.
python -m discovery_lab.unified_cli run --plan examples/approved-work/approved-plan.json \
  --state /tmp/another-m5
```

Opt-in S3: `--s3-bucket`, `--s3-endpoint`, `--s3-prefix`. Uses the caller's AWS
credential chain. **Never** include credentials in manifests or Git.
The local S3-like HTTP tests are not a true AWS/MinIO capability probe.

## Handoff contracts

`selected-work-v1` is an immutable, source-adapter-authorized input. Required:
work identity, title, author, `source_url`, language, tags, all chapter ordinals,
chapter titles, **every selected page** in order and a verifiable content source.
Each page has EITHER `fixture_file` (approved local file) OR
`raw_key`+`raw_sha256` (immutable, externally uploaded S3 RAW). The plan is
hashed byte-for-byte; a conflicting plan for the same work is rejected by the
same ledger instead of overwriting source evidence.

`parsed-page-v1` is immutable per page and parser version, written after RAW
readback. Reparse does not call the upstream site. Incorrect schema, missing
text, mismatched page identity, corrupt byte hashes or missing page cause
`NEEDS_REVIEW`. Publication is forbidden unless every selected page is PARSED.

`verified-source-handoff-v1` is a **read-only** immutable JSON receipt with:
selected manifest hash, work+revision IDs, chapter count/metadata, raw/parsed
keys+hashes and verified published `artifact_manifest_key`.
It deliberately has `translation_started=false`,
`requires_actor_confirmation=true`, and `publication_permitted=false`.
The output only confirms the **selected manifest**, not whole-site coverage.

### Integration boundaries (no silent cross-project writes)

* **Erida / ararra:** the owner of actual source jobs remains
  `source_collect_jobs`+`source_collect_items`, the actor/lease boundary,
  page-selection SHA and revision-aware stage/confirm APIs. The handoff
  may be used by a **separate authenticated adapter** to retrieve selected
  chapter Markdown only after verifying the immutable manifest; this
  component never creates book chapters, starts translation, or publishes.
* **Literotica:** reuse the existing M4 catalog index, chapter/series graph,
  stable author/series inventory, streaming publication and CAS; only a
  site-specific, policy-approved parser and discovery adapter may populate
  an approved manifest. A work linked into several series must retain all
  memberships. Handoff does not update the global index automatically.
* **Rust**: the bounded `mpsc` 8-slot / 4-fetch / 4-upload transport can
  produce RAW objects for the `raw_key` branch; it must supply a page
  identity and exact SHA-256 via its own verified receipts. This Python
  orchestration does not import Rust internals.

## Operational guarantees / limits

- At-least-once stages with conditional immutable object writes; S3 and
  SQLite **cannot** participate in one atomic transaction. An interrupted
  RAW write before local commit is retried by inspecting the deterministic
  raw key and comparing its bytes. Stale lease tokens may not commit.
- No silent success on schema drift, lost or corrupt RAW, duplicate/missing
  pages or incompatible catalog; unresolved stages block publication.
- The currently selected fixture adapter enforces `https://fixture.invalid`,
  UTF-8 and a 1-MiB per-page cap. There is **no live site fetcher/parser**,
  no authorization to bulk-collect Literotica, no CAPTCHA/paywall bypass.
- Replay of the same manifest is idempotent. A changed manifest currently
  requires a new ledger path; future production adapter needs explicit
  revisioned job identities, retention and actor authentication.
- Streaming assembly reads parsed pages one at a time and reuses M4's disk
  spool; individual RAW/parsed receipts still require <=1 MiB memory.
- Failures become `NEEDS_REVIEW`. A human or trusted adapter must decide when
  to retry a rejected source; no automatic bypass or third-party crawling.

## Remaining implementation work

1. Connect a policy-verified real HTTP source adapter (or the approved Erida
   `PublicHtmlFetcher` with its SQLite limiter) and a Literotica DOM snapshot
   parser. No access to the real site is enabled until its robots/ToS gate.
2. Convert site-specific discovery/TOC/series links to `selected-work-v1`
   with preserved provenance and coverage receipts.
3. Add authenticated, explicit RE-02 stage/confirm adapter; independently
   validate source revisions, actor permissions and source quotas.
4. Add Literotica M4 catalog/index update adapter with CAS and no false
   completeness claims; retain source tags, categories and series relations.
5. Add opt-in real MinIO test of the **whole M5 chain**, encrypted secrets,
   retention and reviewable deployment controls. Earlier PR #726 proves
   the separate bounded transport with MinIO, **not** M5 end-to-end.

## M6: actual Erida import handoff (read-only unless the member acts)

M5's published `verified-source-handoff-v1` and the 64-digit SHA-256 are
validated **against all immutable RAW/parsed receipts**, the content-addressed
publisher manifest and individual chapter objects. Any mismatch, omission or
ambiguity fails closed. M6 then creates deterministic, bounded HTML batches
that Erida's existing `POST /v1/imports/preview` / `POST /v1/imports/stages`
accept. No new Erida parser or book import service is invented.

```sh
python -m discovery_lab.unified_cli demo --state /tmp/demo-m6 \
  --erida-out /tmp/demo-m6-import
# The first command returns the batch receipt with the exact file names.
# M6 never stages, confirms, translates or publishes by itself.
```

To validate each batch against an authenticated **trusted** Erida instance
(with explicit consent to send that text to that instance):

```sh
ERIDA_API_KEY=... python -m discovery_lab.unified_cli preview-erida \
  --state /tmp/demo-m6 \
  --erida-manifest /tmp/demo-m6-import/demo-story-<revision-prefix>-import.json \
  --erida-origin https://YOUR-ERIDA-SERVER
```

The `preview-erida` command sends ONLY `POST /v1/imports/preview`. It never
follows redirects (avoids credential leakage), refuses non-HTTPS origins
except loopback, validates the number and order of detected chapter numbers,
and returns only counts and status, not confidential chapter text or keys.
Actual staging uses Erida's existing authenticated actor-owned `/stages` and
`/confirm` endpoints **only by separate explicit human action**.

Limits: each HTML batch < 12 MiB (10 MiB target), individual source chapter
<= 2 MiB for the current M5 small-object sink. Any larger chapter requires a
future streaming M6 reader; splitting one source chapter silently is forbidden.
M6 checks only the explicitly selected plan, never asserts complete site coverage.

Pinned real Erida import parser (`Lalalalendia/ararra@0b793ed1`) is read and
run in isolated GitHub CI to guard against drifting chapter heading formats.

