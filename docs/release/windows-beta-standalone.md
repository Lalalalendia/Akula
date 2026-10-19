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
It creates temporary **upstream sentinel** directories and registry values
on a **GitHub-hosted ephemeral runner**, then installs, opens, restarts and
uninstalls Akula and ensures those sentinels and the local vault survive.
It does **not** read, install or run the actual upstream Aquilum.

## Not proven / release HOLD

- User edits through the installed GUI (the first smoke verifies launch,
  persistent window state, local document unchanged and uninstall, not
  keyboard input or File Gate power-loss recovery).
- Real Windows 10/11 physical-device testing and accessibility.
- Signed Akula `latest.json` and its own Minisign key: the updater remains
  fail-closed. No in-app update/rollback is enabled.
- Authenticode: unsigned candidate may receive SmartScreen or
  Smart App Control warnings/blocks; do not bypass endpoint protections.
- Installer crash during writing/upgrading and true second-generation
  Akula-to-Akula update with rollback.
- Distinct icon art and consistent Mac/Linux IDs remain separate work.

Do not merge/release unless exact-commit Windows NSIS and independent legacy
sentinel acceptance pass. GitHub upload-artifact is a **test artifact**,
never equivalent to a published GitHub Release.
