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
  assert.deepEqual(config.binaries, [{ path: 'akula-app', main: true }]);
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
  for (const value of [
    'scripts/windows-beta-gui-input.ps1',
    'scripts/test-windows-beta.ps1',
  ]) {
    assert.ok(workflow.includes(value), 'missing Windows CI path: ' + value);
  }
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
    'Only QuickOpen, SelectAll and Copy are authorized',
    'if (key != 0x41 && key != 0x43 && key != 0x4f)',
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
