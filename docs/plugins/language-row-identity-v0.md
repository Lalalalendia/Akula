# LANG-01b: row identity evidence and conservative reattachment

Status: draft, stacked on PR #1004. This is not yet a user-facing feature.

## Problem
Markdown rows do not have stable IDs. Row offsets or question strings cannot safely preserve review history after sorting, duplicate rows or semantic edits.

## Contract
A trusted host provides fixed vault ID, note ID and table ID and stores a prior row registry separately from Markdown. This pure, read-only helper calculates length-prefixed BLAKE3 digests of complete cells, then reconnects a saved ID only if the digest appears once on both sides. Sorting and insertion preserve unchanged row IDs. Edited or duplicated rows stay on HOLD and must be explicitly resolved. Different sources and corrupt registries fail closed. Digests do not serve as IDs.

## Gaps before LANG-01 acceptance
The host has not yet minted stable table/row IDs, persisted the registry, verified snapshots through DocumentHub or wired a table menu. This helper must never be called with unverified user-supplied source IDs. Nothing here modifies Markdown or transfers review history automatically for changed rows.

Required CI: Rust workspace tests and exact-head mandatory PR gates. Parent PR #1004 must be accepted first.

Spec: https://app.notion.com/p/41b32a84beec813183d6e615413bae8a
Roadmap: https://app.notion.com/p/41b32a84beec812889dde811903385b5
