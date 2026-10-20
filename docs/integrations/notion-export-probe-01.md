# NOTION-EXPORT-PROBE-01 — offline read-only diagnostic

Status: experimental, opt-in, not an importer. Plan: https://app.notion.com/p/3ff32a84beec815fbec8e1fa26d3e48f

## Run
Export your Notion workspace via the official UI and extract the ZIP in an isolated, private directory. This tool does NOT extract ZIP or modify Akula data.

    cd aquilum-app
    cargo run -p aquilum-core --locked --features notion-export-probe --bin akula-notion-export-probe -- /path/to/extracted-export > inventory.json
    cargo test -p aquilum-core --locked --features notion-export-probe notion_export

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
CI must confirm feature-gated compilation and execution; until then, do not mark tests passed. The next milestone is production-grade ZIP scanning, verified raw-object graph, then File Gate publication; each requires separate review.