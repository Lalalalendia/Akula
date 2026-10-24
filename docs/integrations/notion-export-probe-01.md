# NOTION-EXPORT-PROBE-01 — offline read-only diagnostic

Status: experimental, opt-in, not an importer. Plan: https://app.notion.com/p/3ff32a84beec815fbec8e1fa26d3e48f

## Run
Export your Notion workspace via the official UI and extract the ZIP in an isolated, private directory. This tool does NOT extract ZIP or modify Akula data.

    cd aquilum-app
    cargo run -p aquilum-core --locked --features notion-export-probe --bin akula-notion-export-probe -- /path/to/extracted-export > inventory.json
    cargo test -p aquilum-core --locked --features notion-export-probe notion_export

Optional read-only destination planning (stdout contains source paths/UUIDs):

    cargo run -p aquilum-core --locked --features notion-export-probe --bin akula-notion-export-probe -- --plan /path/to/export.zip > plan.json

The plan uses stable 32-hex Notion UUIDs for proposed note filenames, preserves Markdown link targets, and lists blockers for missing/duplicate IDs, unresolved links, images, CSV schema, HTML and other unsupported entries. No original text is copied to the plan; no files are created. Even a structurally valid plan has publication_supported=false.

Optional **read-only verified Markdown staging** (JSON receipt contains IDs, paths, hashes, byte counts, but never Markdown body):

    cargo run -p aquilum-core --locked --features notion-export-probe --bin akula-notion-export-probe -- --verify-content /path/to/export.zip > receipt.json

This command scans the source, requires a blocker-free plan, reads Markdown into bounded memory, validates BLAKE3 against the first inventory and independently rechecks the full source inventory. ZIP decoding uses the same strict CRC/entry checks. It does not extract, save or import anything. For safety, this first in-memory stage refuses ZIP inputs over 64 MiB and combined Markdown over 32 MiB (each text entry is also bounded by ScanLimits); larger real exports require a future streaming audited reader. Source directories **must be isolated immutable snapshots**: a malicious concurrent writer can still race path checks, and OS-pinned directory handles are not yet implemented.

**No File Gate or Vault publication is authorized.** Typed database schemas, media, Notion permissions, workspace completeness and user approval remain independent gates. A verified receipt is not a successful migration.

JSON goes to stdout. The report can include sensitive original page names and IDs; keep it private.

## What the current module actually does
- Inspects an extracted directory only, rejecting symbolic links and irregular files; hashes regular files with BLAKE3.
- Inventories Markdown, CSV, HTML, unknown attachments and nested ZIP (HTML/ZIP contents remain unparsed).
- Extracts trailing Notion-like UUIDs from file names when present, detects duplicate identities, parses Markdown links/images and computes a basic UUID-based inbound link report. No guessing targets from non-unique titles.
- CSV parser counts rows and columns, including quoted commas/newlines/escaped quotes; no restoration of property types, relations or views.
- Produces deterministic JSON, with unresolved and ambiguous links explicit, and workspace completeness always false.

## Limits and security boundary
Limits: 100000 entries, 2 GiB total bytes, 16 MiB per text file, 64 nested levels, 1 million links, 4096 CSV columns, 1 million CSV rows. Refuses malformed CSV or invalid text encoding.
Files are read-only and no network access, Notion API credentials, Akula File Gate writes, history mutations or search indexing occur.
A directory modified concurrently can present a TOCTOU race. Use an isolated immutable test export; production scanning needs pinned OS handles.
No ZIP extraction, HTML block parsing, Notion rich-text mentions, source permission verification, database model restoration, incremental sync, or statement of full coverage. This is a preliminary diagnostic, not a data backup.

## Gates
Rust unit tests: UUID-linked pages with duplicate titles, unresolved and ambiguous IDs, CSV escaping/newline, size rejection, symlink rejection and nested ZIP classification.
Historical ZIP CI #39586838554 (11/11 tests) passed on 001d1cc7; the reconciled mainline branch requires a new exact-head CI run. The next milestone is production-grade ZIP scanning, verified raw-object graph, then File Gate publication; each requires separate review.

## ZIP input experiment (read-only diagnostic, not an importer)
The opt-in CLI now accepts either an extracted folder or a **top-level .zip** file. The ZIP reader keeps all content in memory under strict bounds and writes nothing; only STORE and DEFLATE with consistent local/central metadata and CRC are permitted. Strictly rejects ZIP64, extra fields, data descriptors, encryption, symlinks, multi-disk and path traversal. Unknown nested ZIPs are inventory entries and are not recursively extracted. Per-entry memory bound is 16 MiB, intentionally restrictive even for large attachments. This early parser is **experimental and not security-audited**; real Notion ZIP compatibility, error recovery and attack-resistance must be independently validated, preferably replacing bespoke ZIP parsing with a reviewed library after dependency and lockfile review.

New tests attempt stored archive, deflate archive, CRC damage, path traversal and size denial. The ZIP revision passed on 001d1cc7; this does not validate the rebased mainline branch or constitute production ZIP security acceptance.
