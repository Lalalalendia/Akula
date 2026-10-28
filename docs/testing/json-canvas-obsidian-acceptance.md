# JSON Canvas / Obsidian bidirectional acceptance

This runbook is an **unexecuted acceptance procedure**. The automated tests cover the
published JSON Canvas reference fixture but **do not establish Obsidian GUI compatibility**.

## Reference corpus

- Source: https://github.com/obsidianmd/jsoncanvas/blob/8ca9fdcac7292a1b44480659d891772a0fa891f7/sample.canvas
- Pinned upstream commit: `8ca9fdcac7292a1b44480659d891772a0fa891f7`.
- Local fixture: `aquilum-app/core/tests/fixtures/jsoncanvas/official-sample.canvas`.
- MIT license: `aquilum-app/core/tests/fixtures/jsoncanvas/LICENSE`.
- Core test: `cargo test -p aquilum-core --test json_canvas_official_reference`, from `aquilum-app`.

## Manual external-app acceptance

1. Create a disposable local Obsidian vault. Copy the pinned `official-sample.canvas` to
   `reference.canvas`; optionally copy the referenced `readme.md`, `spec/1.0.md` and
   `_site/logo.svg` into matching paths if file previews are important. Missing referenced
   files must not trigger network access or edits to the Canvas source.
2. Record SHA-256 of `reference.canvas` before opening. Open it in Obsidian Canvas, verify
   the group, text, file placeholders/previews, the directed edge and z-order. Close without
   editing. Verify the resulting file and record any Obsidian-initiated rewrites separately.
3. Open the same disposable vault with an installed Akula build; open the Canvas and make
   **only one text-card move or text edit**. Confirm File Gate saves, undo/redo and the
   expected revised on-disk JSON. Inspect that all file paths, original nodes, IDs and
   directed edge fields remain unchanged.
4. Close Akula, reopen `reference.canvas` in Obsidian. Check it parses, the edited text
   appears, original references and edge geometry survive, and no warnings/loss are shown.
5. In Obsidian, add another group, card and edge; close it. Reopen in Akula. Verify those
   new elements render (or are explicitly marked unsupported), and that merely opening
   and closing does not mutate the source bytes.
6. Repeat with Obsidian-specific plugin properties and a Canvas with unresolved
   file/subpath references. For unsupported node data, require preserved JSON, not guessed
   application behavior. Test an external edit while Akula has a dirty session: Akula
   must reject overwrite rather than silently replace the external revision.
7. Attach before/after files, SHA-256 values, Obsidian version, Akula build/OS, screenshots,
   any logs, and a PASS/FAIL per scenario to the C5 acceptance task in Notion.

**Do not mark C5 complete** until actual two-way application execution has evidence.
Test only disposable copies; the procedure is not a production data migration.
