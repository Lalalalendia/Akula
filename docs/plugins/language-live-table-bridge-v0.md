# LANG-01c — live editor table bridge (read-only)

Status: Draft; neither a working Languages UI nor a plugin runtime.

Native TextEditor::study_table_snapshot returns the currently active parsed table from the live editor document, including unsaved changes. It does not read a file or write to Markdown.

The Core capture contract bounds rows (4096), columns (64) and total cell text (1 MiB). It refuses invalid document ranges, malformed shapes and merged cells rather than generating ambiguous questions.

The offset range is ephemeral to one editor revision, not a persistent table ID or a permission grant. The trusted host still needs stable note/table identities, sidecar mapping and fresh access checks before persisting membership or reading exercises.

Six pure Core tests check source invariance, malformed ranges and merges, bounds, oversized Unicode text and full-byte value preservation. Pending native integration tests and exact-head CI.

Next: trusted host binding for the active note and table; then menu option to add a table to collection and column-mapping dialog. Existing File Gate/DocumentHub remain the only valid writers.