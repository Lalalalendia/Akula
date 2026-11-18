# LANG-01d: conservative table-anchor sidecar

Status: Draft, not a completed source identity implementation.

## Trust boundary
A trusted host supplies stable vault/note identity and current authorized parsed tables. Neither path strings, a guessed table offset, nor content fingerprints are authorization. The host must still mint stable note/table IDs and persist trusted note rename/move events.

## Reconciliation contract
- Each table has a host-issued ID and a full-table BLAKE3 fingerprint based on length-prefixed UTF-8 cells. The fingerprint is evidence, not a durable ID.
- Reuse a table ID across sorting and insertion only when its fingerprint appears exactly once among old anchors and exactly once in the current note.
- Any edited, deleted or duplicated table remains HOLD. Table positions, header names and partial content never silently override that rule.
- Validate note identity, table ID/index uniqueness, budgets and structural correctness before processing. Do not persist raw table cells in sidecar.

## Limitations
This pure step has no host-based note ID issuer, no persisted sidecar, and no GUI. Edits will currently put the whole table on HOLD until a verified editor delta or user confirmation can reconcile. This deliberately favors safety but needs user-facing recovery.

## Acceptance
Eight tests cover reorder, insertion, edits, duplicates, note/vault mismatch, malformed registry, Unicode, budget and unchanged logical note identity. Need exact-head Rust tests, native test and protected OS CI. Subsequent PR must persist sidecar with revision CAS without changing source Markdown.