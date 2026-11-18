# LANG-01c UI — native table study preview

Status: draft user-visible read-only slice, not LANG-01/02 completion.

UX: right-click a Markdown table in the active native editor, select Study cards: preview, then choose a question column and a different answer column by heading. A dialog shows ready/skipped counts and a few bounded examples. Copy pairs exports up to 64 complete question/answer pairs as TSV.

Safety: preview reads only the existing TextEditor live model (including unsaved edits), never rereads disk or writes Markdown. Merged/ragged/oversized cells or invalid ranges fail closed. No collection membership, stable ID grant, review progress, network or third-party plugin code is involved.

Existing persistent document UUIDs from the native UI session (workspace_id and Session::document/renamed) should be reused by the future host integration. This UI does not persist a table offset as an ID.

Tests: four native pure tests cover arbitrary headings, reversal, invalid rows, and display bounds without copy truncation. Mandatory exact-head CI and installed Windows GUI acceptance remain required.