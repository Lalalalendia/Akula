# LANG-02e — native named collection creation

Stacked on PR #1005 until that PR is merged. After choosing the question/answer columns in the current Markdown table, select "New collection…" in the native collection menu, type a unique name, then choose "Create and add". The resulting collection receives a random stable UUID and is atomically bound to the active table ID and source column mapping in the same metadata SQLite transaction.

- Existing collections remain selectable, one source table can join multiple named collections.
- Trim names; reject empty, >64 Unicode scalar values, >256 UTF-8 bytes, control and Unicode direction-override characters. Check duplicate names with a case-insensitive comparison before binding; the Core transaction has a second uniqueness guard to avoid races.
- A source note must report clean, acknowledged saved state through `Note::clean_cache_hash`; unsaved or conflicted notes are rejected rather than creating metadata bindings to unsaved words. Recheck again at commit.
- No Markdown mutation, language-specific schema assumptions, third-party plugins, or new file-system authority.
- Unit tests: Unicode names, invisible chars and length bounds, case-insensitive duplicate names; existing Core tests also exercise mapping, identity and atomic CAS.
- Await exact-head CI and installed Windows GUI validation before merging. No release/deployment.