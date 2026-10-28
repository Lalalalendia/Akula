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

## Reproducible two-app evidence collector (interactive Windows 10/11)

This is a guided **real GUI acceptance procedure**, not a completed CI run.
Obsidian CLI needs the 1.12.7+ installer and must be enabled in Obsidian
Settings > General. It must connect to a running Obsidian Desktop; Obsidian
Headless is not equivalent to a Canvas view.

From a repository checkout with Windows PowerShell 5.1, Node.js and installed
Akula, prepare a genuinely new disposable vault:

```powershell
$vault = Join-Path $env:TEMP "akula-c5-$(Get-Date -Format yyyyMMddHHmmss)"
powershell.exe -NoProfile -Sta -ExecutionPolicy Bypass -File scripts/windows-canvas-obsidian-crossapp.ps1 -Mode Prepare -VaultPath $vault
```

Register this exact folder as a vault in Obsidian, turn on CLI, and keep
Obsidian running. Locate the official `Obsidian.com` beside
`Obsidian.exe` (path varies by installation). Start the collector
with a new evidence directory *outside* the vault:

```powershell
$cli = Join-Path $env:LOCALAPPDATA "Programs\Obsidian\Obsidian.com"
$proof = Join-Path $env:TEMP "akula-c5-proof-$(Get-Date -Format yyyyMMddHHmmss)"
powershell.exe -NoProfile -Sta -ExecutionPolicy Bypass -File scripts/windows-canvas-obsidian-crossapp.ps1 -Mode Run -VaultPath $vault -ObsidianCli $cli -ObsidianVaultName (Split-Path $vault -Leaf) -EvidenceDir $proof
```

The collector refuses a non-disposable vault, a symlinked source, a wrong
Obsidian vault, an existing output folder, a missing installed Akula or a
missing real Obsidian Desktop process. It never deletes or resets a vault.

1. Obsidian opens `reference.canvas`: inspect five objects and one
   edge. The script checks the native Canvas view and DOM, captures a GUI
   screenshot and verifies opening did not rewrite the original SHA-256.
2. It closes that Obsidian view and launches **installed Akula** on the same
   file. In Akula, click `+ Text` exactly once and wait for File Gate
   to persist the new node. The script checks UIA counts, saves the new file
   and captures a screenshot.
3. Obsidian opens Akula's six-node revision. In **Obsidian's GUI**, add one
   group, one text card, and one new edge touching the new card, then wait
   for save. The script captures that real Canvas view and revision.
4. Installed Akula opens the final eight-node/two-edge revision. The script
   records UIA and screenshot evidence, and rejects a read-only rewrite.
5. The independent Node validator compares three saved Canvas files,
   unchanged original IDs, field contents, relative layer order, file paths
   and connection, both applications' additions, five GUI observations,
   screenshots, versions and SHA-256 values. Missing evidence fails closed.

The evidence directory contains `00-original.canvas`,
`01-akula.canvas`, `02-obsidian.canvas`,
`report.json`, and five PNG screenshots. Human review of images
is still mandatory for connection visibility, geometry, text, missing file
placeholders, and z-order; UIA counts alone are not pixel proof.


### Machine evidence is not visual acceptance

At the end of the guided collector, the Node validator prints
`AKULA_C5_OBSIDIAN_MACHINE_EVIDENCE_PASS` and
`AKULA_C5_OBSIDIAN_VISUAL_REVIEW_PENDING`. **Neither of these
markers permits C5 to be marked Cross-app verified.** Hashes, PNG headers
and DOM/UIA counters do not establish that cards, text, file placeholders,
edge geometry and layer ordering actually look correct.

Open and inspect **all five PNGs** in the generated evidence directory, and
review the three saved Canvas files. When the screenshot or file inspection
finds any problem, record FAIL in Notion and retain the evidence; do not
write a passing review. After all checks pass, create
`visual-review.json` *in the evidence directory* with the schema:

```json
{
  "reviewer": "name or review identifier",
  "reviewedAtUtc": "2026-10-28T12:00:00Z",
  "phases": {
    "obsidian_initial": { "screenshotSha256": "<copy from report.json>", "visualPass": true },
    "akula_edited": { "screenshotSha256": "<copy from report.json>", "visualPass": true },
    "obsidian_reopened": { "screenshotSha256": "<copy from report.json>", "visualPass": true },
    "obsidian_edited": { "screenshotSha256": "<copy from report.json>", "visualPass": true },
    "akula_reopened": { "screenshotSha256": "<copy from report.json>", "visualPass": true }
  },
  "checks": {
    "readableText": true,
    "visibleObjects": true,
    "visibleConnections": true,
    "groupLayering": true,
    "filePlaceholders": true
  }
}
```

The timestamp is an example; replace it with the actual review time. Copy
each phase's exact screenshot SHA-256 from `report.json`. Never
write `visualPass: true` or any positive check without inspecting
the corresponding image.

Rerun:

```powershell
node scripts/canvas-crossapp-validate.cjs $proof aquilum-app/core/tests/fixtures/jsoncanvas/official-sample.canvas
```

Only then may the validator emit `AKULA_C5_OBSIDIAN_CROSSAPP_PASS`
bound to the exact screenshot digests. Human attestation is evidence of a
review, not independent proof of GUI execution. Keep versions, screenshots,
source revisions and a reviewer reference in Notion.

The guided GUI process does **not** run in required GitHub CI. The separate
Node validator unit tests can run without Obsidian and never establish
two-app compatibility by themselves:

```bash
node --test .github/scripts/canvas-crossapp-validate.test.cjs
```

Windows 7 physical Akula acceptance remains a different requirement;
an Obsidian Windows 10/11 run does not prove Windows 7 compatibility.
