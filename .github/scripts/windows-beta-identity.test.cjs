'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.resolve(__dirname, '../..');
const read = p => fs.readFileSync(path.join(root, p), 'utf8');
const app = read('aquilum-app/native/Cargo.toml');
const config = JSON.parse(read('aquilum-app/native/packaging/akula-windows.json'));
const workflow = read('.github/workflows/windows-beta.yml');
const script = read('scripts/test-windows-beta.ps1');
const workspace = read('aquilum-app/native/src/workspace.rs');
const autostart = read('aquilum-app/native/src/autostart.rs');
const instance = read('aquilum-app/native/src/instance.rs');

test('Windows packager has a distinct, version-pinned Akula identity', () => {
  const cargoVersion = app.match(/^version = "([^"]+)"/m)?.[1];
  assert.ok(cargoVersion);
  assert.equal(config.version, cargoVersion);
  assert.equal(config.productName, 'Akula');
  assert.equal(config.name, 'aquilum-native', 'must avoid cargo-packager 0.11.8 trying to chdir into JSON file');
  assert.equal(config.identifier, 'io.github.lalalendia.akula');
  assert.notEqual(config.identifier, 'com.dmitriy.aquilum-app');
  assert.deepEqual(config.binaries, [{ path: 'akula-app', main: true }]);
  assert.equal(config.homepage, 'https://github.com/Lalalalendia/Akula');
  assert.equal(config.nsis.installerIcon, '../icons/icon.ico');
  assert.deepEqual(config.icons, [
    '../icons/32x32.png', '../icons/128x128.png',
    '../icons/128x128@2x.png', '../icons/icon.ico',
  ]);
});

test('Windows runtime data and global process identities do not alias Aquilum', () => {
  assert.match(workspace, /#\[cfg\(target_os = "windows"\)\]\s*pub const APP_ID: &str = "io\.github\.lalalendia\.akula";/);
  assert.match(workspace, /#\[cfg\(not\(target_os = "windows"\)\)\]\s*pub const APP_ID: &str = "com\.dmitriy\.aquilum-app";/);
  assert.match(workspace, /#\[cfg\(not\(target_os = "windows"\)\)\]\s*if data_dir\.ends_with\(APP_ID\)/);
  assert.match(autostart, /#\[cfg\(target_os = "windows"\)\]\s*const NAME: &str = "Akula";/);
  assert.match(instance, /pub const WINDOW_CLASS: &str = "AkulaWindow";/);
});

test('candidate builds local-only, uses explicit Windows config and never replaces upstream', () => {
  for (const str of [
    'runs-on: windows-latest',
    'cargo build --release --locked -p aquilum-native --features installed',
    '--config native/packaging/akula-windows.json',
    '--binaries-dir $release',
    "Join-Path $release 'akula-app.exe'",
    './scripts/test-windows-beta.ps1',
    'Upload candidate (not a release)',
  ]) {
    assert.ok(workflow.includes(str), 'missing workflow contract: ' + str);
  }
  // YAML comments are descriptive: enforce the *active* release commands.
  const activeLines = workflow.split('\n').filter(line => !/^\s*#/.test(line)).join('\n');
  assert.doesNotMatch(activeLines, /Freaction\/Aquilum|--all-features|gh release create|--features installed,sync-installed-host/);
});

test('installer smoke checks distinct directories and preserves existing Aquilum', () => {
  for (const str of [
    "'github-hosted'",
    "'Aquilum'",
    "'Akula'",
    "'com.dmitriy.aquilum-app'",
    "'io.github.lalalendia.akula'",
    "'AQUILUM-INSTALL'",
    "'AQUILUM-DATA'",
    'Assert-LegacyUntouched',
    "Get-FileHash -LiteralPath $note",
    'Start-And-Close',
    "'uninstall.exe'",
  ]) {
    assert.ok(script.includes(str), 'missing smoke contract: ' + str);
  }
  assert.doesNotMatch(script, /gh release download|Freaction\/Aquilum|Remove-Item .*Aquilum/);
});
