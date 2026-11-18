# LANG-00 / LANG-01 — read-only table study preview

**Status:** first Core slice for review. No GUI, plugin runtime, persistent source IDs or games.
**Spec:** [Akula Languages](https://app.notion.com/p/41b32a84beec8100a0acfe7b5c7949b7) and [roadmap](https://app.notion.com/p/41b32a84beec812889dde811903385b5).

## Core audit at main@710e9627 (2026-11-18)

- The native Markdown table editor already has parsing, cell editing and merged-cell operations in aquilum-app/native/src/ui/editor/table/. Reuse it instead of adding a second Markdown parser.
- The existing editor highlight module implements syntax colors, NOT durable user color-range annotations and popups.
- The Core File Gate / DocumentHub implement guarded file snapshots, dirty buffers and conflict-aware writes. All future Language Pack writes must use this path.
- Core plugins have typed manifest, permission and bounded read primitives, but this is NOT a complete third-party plugin runtime.
- Plain Markdown table rows have no stable identifiers. A row index or content hash cannot safely identify a learning item after sorting, editing, duplicate rows or moving files.

## What this PR actually adds

Core learning::preview_table receives only an in-memory, already-parsed table snapshot supplied by an authorized trusted host. The caller explicitly maps the question column, one or more accepted-answer columns and optional context column. The preview has a physical-row scan limit and reports malformed rows. It does not infer synonyms or split comma-separated values.

Supported examples: EN/RU; EN/RU1/RU2/context; definition/term with reversed column mapping.

There are no document writes, filesystem reads, arbitrary guest calls, durable table/row IDs, collections, flashcards, highlights, prompts or scheduler in this slice. No UI button exists yet. The source_row field is ephemeral and MUST NOT key a persisted review history.

## Next gates

1. Define stable document/table/row identity with a sidecar registry and explicit HOLD for ambiguous matches, including duplicate rows, note rename and external edits.
2. Wire the native table view to this preview without dirty-buffer loss and with explicit column selection.
3. Create multi-table collection membership without copying source values.
4. Add a shared, non-destructive text-annotation layer and popup linked to a verified source value.
5. Generate exercises and store review history only after identity and ambiguity safeguards.

Test command from aquilum-app: cargo test -p aquilum-core learning::tests --locked

Exact-head Main PR gate and Application checks must pass. A passing Core test is not evidence of user-facing Windows GUI acceptance or guest sandbox safety.
