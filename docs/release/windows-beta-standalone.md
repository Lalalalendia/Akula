# Akula Windows Beta — independent installer boundary (2026-10-19)

This is an **unpublished candidate**, not an Akula Release.

## Existing product must be preserved

The upstream Freaction/Aquilum product is a separate application. Its
NSIS uninstall key `HKCU/.../Uninstall/Aquilum`, autostart value `Aquilum`,
installation under `%LOCALAPPDATA%/Aquilum` and data at
`%APPDATA%/com.dmitriy.aquilum-app` are **not** Akula migration targets.

The Windows candidate instead packages:

- product / NSIS uninstall entry: `Akula`;
- installer identity and default per-user location: `Akula`;
- installed executable: `akula-app.exe` (Rust package remains `aquilum-native`);
- runtime AppData: `%APPDATA%/io.github.lalalendia.akula`;
- Windows autostart name: `Akula`;
- Windows window class: `AkulaWindow`.

Mac/Linux packaging retains its existing legacy configuration in this isolated
Windows PR; platform-wide branding requires a separate reviewed patch. The
provisional Windows installer icon still uses the inherited icon assets.

The `windows-beta.yml` workflow builds only `--features installed`,
never enables `sync-installed-host`, S3 mutation or a release publication.
It uses a separate cargo-packager configuration with immutable product ID.
The JSON **must** include `"name": "aquilum-native"`: cargo-packager 0.11.8
otherwise calls `set_current_dir` on the JSON file itself while inferring a package
name, triggering Windows `os error 267` before NSIS. The explicit `name` bypasses
that version-specific bug, and matches the `-p aquilum-native` selection.
The standalone config lives under `native/packaging`, so icon paths are
explicitly relative (`../icons/...`). `binariesDir` is explicitly set **in JSON** to the compiled Cargo release directory,
while `--out-dir` is only the NSIS output. In cargo-packager 0.11.8 the parsed
`--binaries-dir` CLI flag is not applied to file-based configs. The JSON field
avoids the broken default which would point at the distribution folder.
It creates temporary **upstream sentinel** directories and registry values
on a **GitHub-hosted ephemeral runner**, then installs, opens, restarts and
uninstalls Akula and ensures those sentinels and the local vault survive.
It does **not** read, install or run the actual upstream Aquilum.

## Not proven / release HOLD

- **Physical user acceptance** on Windows 10/11 and accessibility. The
  hosted runner now additionally attempts `--open --focus`, foreground-bound
  Windows `SendInput` with layout-mapped virtual key presses, a real GUI edit, timed on-disk
  persistence, and a second installed launch preserving exact bytes. This is
  a stronger automated test but is **not** a physical user walkthrough, a
  power-loss/crash-recovery proof, or evidence of editor content being
  visually correct after restart. It is only accepted after exact-head CI
  success; the test never writes the Markdown file after fixture setup.
- Real Windows 10/11 physical-device testing and accessibility.
- Signed Akula `latest.json` and its own Minisign key: the updater remains
  fail-closed. No in-app update/rollback is enabled.
- Authenticode: unsigned candidate may receive SmartScreen or
  Smart App Control warnings/blocks; do not bypass endpoint protections.
- Installer crash during writing/upgrading and true second-generation
  Akula-to-Akula update with rollback.
- Distinct icon art and consistent Mac/Linux IDs remain separate work.

## Hosted Windows editor smoke (not a release gate substitute)

`.github/workflows/windows-beta.yml` runs `scripts/test-windows-beta.ps1`
with the narrowly scoped `scripts/windows-beta-gui-input.ps1` helper.
The helper refuses to run outside a GitHub-hosted Windows x64 runner,
identifies the foreground Akula window by native HWND **and process ID**,
then sends ordinary virtual-key events (not Unicode `VK_PACKET`) to the `--focus` editor.
The smoke fails rather than appending text directly to the note when OS
input is unavailable. A unique marker must reach the actual disk file
within 30 seconds, while the original heading and body survive.

A successful run checks: installed package and executable hash; legacy
Aquilum directory, uninstall and Run key untouched; GUI edit/save;
graceful close and reopened note with byte-identical local document;
uninstall leaving the modified document and Akula user state intact.

This deliberately does **not** prove native first-frame rendering of the
reopened text, screen-reader usability, Windows 10 hardware behavior,
immunity to hostile same-user writers, OS power-loss atomicity, trusted
installer signatures, or a production update/rollback channel. Those
remain separate release HOLDs; a CI artifact is not a public installer.

Do not merge/release unless exact-commit Windows NSIS and independent legacy
sentinel acceptance pass. GitHub upload-artifact is a **test artifact**,
never equivalent to a published GitHub Release.
