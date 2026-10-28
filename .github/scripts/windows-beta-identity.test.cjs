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
const guiInput = read('scripts/windows-beta-gui-input.ps1');
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
  assert.deepEqual(config.binaries, [{ path: 'akula-app', main: true }, { path: 'shiva-pdf-preview', main: false }]);
  assert.match(workflow, /shiva-pdf-preview\.exe/, 'Windows beta must bundle the native PDF companion');
  assert.equal(path.resolve(root, 'aquilum-app/native/packaging', config.binariesDir),
    path.resolve(root, '.artifacts/cargo-native/release'),
    'cargo-packager 0.11.8 takes binariesDir from JSON; CLI flag is ignored');
  assert.equal(config.homepage, 'https://github.com/Lalalalendia/Akula');
  assert.equal(config.nsis.installerIcon, '../icons/icon.ico');
  assert.deepEqual(config.icons, [
    '../icons/32x32.png', '../icons/128x128.png',
    '../icons/128x128@2x.png', '../icons/icon.ico',
  ]);
});

test('Pixel shark icon is installed consistently in Windows EXE, NSIS and desktop packages', () => {
  const resource = fs.readFileSync(path.join(root, 'aquilum-app/native/assets/icon.ico'));
  const installer = fs.readFileSync(path.join(root, 'aquilum-app/native/icons/icon.ico'));
  assert.deepEqual(resource, installer, 'native EXE and NSIS must use the same shark artwork');
  assert.equal(resource.readUInt16LE(0), 0);
  assert.equal(resource.readUInt16LE(2), 1, 'Windows ICO image type');
  assert.ok(resource.readUInt16LE(4) >= 6, 'include multiple Windows taskbar resolutions');

  const icons = [
    ['aquilum-app/native/assets/icon-64.png', 64],
    ['aquilum-app/native/icons/32x32.png', 32],
    ['aquilum-app/native/icons/128x128.png', 128],
    ['aquilum-app/native/icons/128x128@2x.png', 256],
  ];
  for (const [filename, size] of icons) {
    const content = fs.readFileSync(path.join(root, filename));
    assert.equal(content.subarray(0, 8).toString('hex'), '89504e470d0a1a0a',
      filename + ' must be a real PNG');
    assert.equal(content.readUInt32BE(16), size);
    assert.equal(content.readUInt32BE(20), size);
  }
  const icns = fs.readFileSync(path.join(root, 'aquilum-app/native/icons/icon.icns'));
  assert.equal(icns.toString('ascii', 0, 4), 'icns');
  assert.equal(icns.readUInt32BE(4), icns.length);
  const nativeBuild = read('aquilum-app/native/build.rs');
  assert.match(nativeBuild, /join\("assets\/icon\.ico"\)/,
    'Windows binary resource should use the new icon.ico');
});

test('Windows runtime data and global process identities do not alias Aquilum', () => {
  // Akula Linux now intentionally shares Akula's independent identity,
  // not Aquilum's AppData. Windows has not changed its product identity.
  assert.match(workspace, /#\[cfg\(any\(target_os = "windows", target_os = "linux"\)\)\]\s*pub const APP_ID: &str = "io\.github\.lalalendia\.akula";/);
  assert.match(workspace, /#\[cfg\(not\(any\(target_os = "windows", target_os = "linux"\)\)\)\]\s*pub const APP_ID: &str = "com\.dmitriy\.aquilum-app";/);
  assert.match(workspace, /#\[cfg\(not\(any\(target_os = "windows", target_os = "linux"\)\)\)\]\s*if data_dir\.ends_with\(APP_ID\)/);
  assert.match(autostart, /#\[cfg\(any\(target_os = "windows", target_os = "linux"\)\)\]\s*const NAME: &str = "Akula";/);
  assert.match(instance, /pub const WINDOW_CLASS: &str = "AkulaWindow";/);
});

test('candidate builds local-only, uses explicit Windows config and never replaces upstream', () => {
  for (const str of [
    'runs-on: windows-latest',
    'cargo build --release --locked -p aquilum-native --features installed',
    '--config native/packaging/akula-windows.json',
    "Join-Path $release 'akula-app.exe'",
    './scripts/test-windows-beta.ps1',
    'Upload candidate (not a release)',
  ]) {
    assert.ok(workflow.includes(str), 'missing workflow contract: ' + str);
  }
  // YAML comments are descriptive: enforce the *active* release commands.
  const activeLines = workflow.split('\n').filter(line => !/^\s*#/.test(line)).join('\n');
  assert.doesNotMatch(activeLines, /Freaction\/Aquilum|--all-features|gh release create|--features installed,sync-installed-host|--binaries-dir/);
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

test('installed Windows beta GUI edit must use OS-level input into its own foreground editor', () => {
  const route = require('./ci-shared-manifest-route.cjs');
  for (const value of [
    'scripts/windows-beta-gui-input.ps1',
    'scripts/test-windows-beta.ps1',
  ]) {
    assert.ok(route.OWNED_SUITES.owned_windows_beta.includes(value),
      'missing required Windows CI owner path: ' + value);
    assert.equal(route.assess({ paths: [value] }).owned_windows_beta, true,
      'Windows GUI source-only change must enter Main PR gate: ' + value);
  }
  assert.ok(script.includes('windows-beta-gui-input.ps1'),
    'installed Windows smoke must still invoke the real OS GUI input helper');
  for (const value of [
    'Start-And-Close -EditNote',
    "if ($EditNote -or $VerifyRestoredContent) { $arguments += ' --focus' }",
    'Send-AkulaBetaText -Process $app -Text $syntheticEdit',
    'AKULA_WINDOWS_BETA_GUI_EDIT_PASS',
    'Restart changed the previously edited note',
    'Uninstall changed the edited local document',
  ]) {
    assert.ok(script.includes(value), 'missing real GUI persistence assertion: ' + value);
  }
  assert.match(guiInput, /RUNNER_ENVIRONMENT.*github-hosted/);
  assert.match(guiInput, /RUNNER_OS.*Windows/);
  assert.match(guiInput, /GetWindowThreadProcessId/);
  assert.match(guiInput, /GetForegroundWindow/);
  assert.match(guiInput, /SendInput/);
  assert.match(guiInput, /VkKeyScanExW/);
  assert.match(guiInput, /MapVirtualKeyExW/);
  assert.match(guiInput, /TypeAsciiKeys/);
  assert.match(guiInput, /KEYEVENTF_KEYUP/);
  assert.match(guiInput, /GetKeyboardLayout/);
  assert.doesNotMatch(guiInput, /KEYEVENTF_UNICODE|TypeUnicode/,
    'VK_PACKET bypasses normal native key translation on current Windows host');
  assert.match(guiInput, /Window PID does not match launched Akula/);
  assert.match(guiInput, /Refusing to type into a nonforeground window/);
  assert.doesNotMatch(guiInput, /\bSet-Content\b|\bAdd-Content\b|\bWriteAllText\b/);
  assert.doesNotMatch(script, /AKULA_WINDOWS_BETA_SMOKE_PASS/,
    'a launch-only smoke must not be mistaken for true GUI editing');
});

test('reopened note GUI readback uses foreground OS shortcuts and clipboard oracle', () => {
  for (const token of [
    'Start-And-Close -VerifyRestoredContent',
    "Set-Clipboard -Value $clipboardSentinel",
    'Send-AkulaBetaShortcut -Process $app -Action SelectAll',
    'Send-AkulaBetaShortcut -Process $app -Action Copy',
    'Get-Clipboard -Raw',
    'Reopened editor copy differs from the exact saved Markdown note',
    'AKULA_WINDOWS_BETA_REOPEN_GUI_READBACK_PASS',
    'Restart changed the previously edited note',
  ]) {
    assert.ok(script.includes(token), 'missing real restored-document GUI assertion: ' + token);
  }
  for (const token of [
    'PressCtrlShortcut(',
    "ValidateSet('SelectAll', 'Copy')",
    'GetWindowThreadProcessId',
    'GetForegroundWindow',
    'Refusing shortcut on nonforeground window',
    'Only selection/copy/search shortcuts are authorized',
    'key != 0x41 && key != 0x43 && key != 0x4F',
    'VK_CONTROL',
    'KEYEVENTF_KEYUP',
  ]) {
    assert.ok(guiInput.includes(token), 'missing bounded OS keyboard shortcut guard: ' + token);
  }
  const middle = script.slice(script.indexOf('if ($VerifyRestoredContent) {'));
  assert.doesNotMatch(middle, /Set-Content -LiteralPath \$note|Add-Content -LiteralPath \$note|WriteAllText\(\$note/,
    'UI readback must never simulate success by writing to the note');
});

test('CI candidate manifest records actual checkout and hashes after the GUI test', () => {
  const provenance = read('scripts/windows-beta-candidate-manifest.ps1');
  const smoke = workflow.indexOf('Install, verify UIA Copy, edit, restart and uninstall beside Aquilum sentinel');
  const manifest = workflow.indexOf('Verify candidate provenance and write manifest');
  const upload = workflow.indexOf('Upload candidate (not a release)');
  assert.ok(smoke >= 0 && manifest > smoke && upload > manifest,
    'candidate may be uploaded only after installed GUI persistence succeeds');
  for (const required of [
    'scripts/windows-beta-candidate-manifest.ps1',
    'dist/candidate-manifest.json',
    'if-no-files-found: error',
    'github.run_id',
    'github.run_attempt',
  ]) {
    assert.ok(workflow.includes(required), 'candidate artifact missing: ' + required);
  }
  for (const required of [
    'git -C $env:GITHUB_WORKSPACE rev-parse HEAD',
    "$env:GITHUB_EVENT_NAME -eq 'pull_request'",
    'Get-FileHash -LiteralPath $setup.FullName -Algorithm SHA256',
    'Get-FileHash -LiteralPath $binary -Algorithm SHA256',
    'Get-FileHash -LiteralPath $lock -Algorithm SHA256',
    'actual_checkout_sha = $source',
    'gui_persistence_smoke',
    'AKULA_WINDOWS_BETA_PROVENANCE_PASS',
  ]) {
    assert.ok(provenance.includes(required), 'provenance invariant missing: ' + required);
  }
  assert.doesNotMatch(provenance, /gh release create|Invoke-WebRequest|Start-Process/,
    'manifest must neither distribute a release nor run other executables');
});

test('Windows beta NSIS apply only accepts signed typed candidate and remains gated', () => {
  const apply = read('aquilum-app/native/src/updater/apply.rs');
  const release = read('aquilum-app/native/src/updater/release.rs');
  const updater = read('aquilum-app/native/src/updater/mod.rs');
  const windowsApply = apply.split('#[cfg(target_os = "windows")]')[1]
    ?.split('#[cfg(target_os = "macos")]')[0];
  assert.ok(windowsApply, 'Windows apply implementation must exist');
  assert.match(windowsApply, /candidate: &super::release::SignedCandidate/);
  assert.match(windowsApply, /if !super::release::channel_ready\(\)/);
  assert.match(windowsApply, /stage_signed_windows_setup\(candidate.bytes\(\), candidate.sha256\(\)\)/);
  assert.match(windowsApply, /ShellExecuteW/);
  assert.match(apply, /file\.sync_all\(\)/);
  assert.match(apply, /if actual != expected_sha256/);
  assert.doesNotMatch(windowsApply, /pub(?:\(crate\))? fn apply\(payload: &\[u8\]/,
    'unsigned arbitrary bytes cannot be a Windows installer argument');
  assert.match(updater, /payload: Arc<Mutex<Option<release::SignedCandidate>>>/);
  assert.doesNotMatch(updater, /Some\(candidate.into_bytes\(\)\);\s*Ok\(\(\)\)/,
    'unsealing an authenticated Windows candidate before apply is prohibited');
  assert.match(release, /pub\(super\) const PUBLIC_KEY: &str = "[^"]*"/);
  assert.match(release, /pub\(super\) const fn channel_ready\(\) -> bool \{\s*cfg!\(all\(target_os = "windows", feature = "installed"\)\)\s*\}/);
});

test('installed large-note replacement uses an isolated, PID-bound Win32 shortcut helper', () => {
  const proof = read('scripts/windows-beta-replace-all.ps1');
  assert.match(script, /Test-AkulaBetaReplaceAll -Executable \$exe -Vault \$vault -DataDir \$akulaData -LineCount \$lines/);
  assert.match(proof, /RUNNER_ENVIRONMENT -ne 'github-hosted'/);
  assert.match(proof, /MainWindowTitle -notlike 'Large paste proof\*'/);
  assert.match(proof, /GetWindowThreadProcessId/);
  assert.match(proof, /GetForegroundWindow/);
  assert.match(proof, /Only synthetic SelectAll\/Copy\/Paste\/Undo\/Redo shortcuts are permitted/);
  assert.match(proof, /AKULA_WINDOWS_REPLACE_ALL_PASS lines=\$LineCount/);
  assert.match(proof, /Ctrl\+Y redo paste/);
  assert.match(proof, /Ctrl\+Y then Ctrl\+Z did not recover exact original source bytes/);
  assert.doesNotMatch(guiInput, /ValidateSet\('SelectAll', 'Copy', 'Paste'/,
    'production readback helper must never gain destructive shortcuts');
});
