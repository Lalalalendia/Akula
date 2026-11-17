'use strict';
// VIDEO-18: fail-closed non-release supply-chain gate. This does NOT sign,
// install, authenticate Windows PE binaries, or authorize publishing.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const { test } = require('node:test');
const path = require('node:path');
const root = path.resolve(__dirname, '../..');
const file = (name) => fs.readFileSync(path.join(root, name), 'utf8');
const config = (name) => JSON.parse(file(name));
const RELEASE = 'aquilum-app/native/packaging/akula-windows.json';
const LAB = 'aquilum-app/native/packaging/akula-windows-video17-lab.json';

function binaries(value) {
  assert.ok(Array.isArray(value.binaries));
  return value.binaries.map((bin) => {
    assert.equal(typeof bin.path, 'string');
    assert.equal(bin.path.includes('/'), false);
    assert.equal(bin.path.includes('\\'), false);
    assert.equal(bin.main === true || bin.main === false, true);
    return bin.path;
  });
}
function validatePackageSplit(release, lab) {
  assert.deepEqual(binaries(release), ['akula-app', 'shiva-pdf-preview'],
    'HOLD: five-binary video package has not passed signed/protected release admission');
  assert.deepEqual(binaries(lab), [
    'akula-app', 'shiva-pdf-preview', 'akula-video-mux',
    'akula-yt-dlp', 'akula-quickjs'
  ], 'VIDEO-17 lab must be the only five-binary NSIS configuration');
  assert.deepEqual(Object.keys(release).sort(), Object.keys(lab).sort());
  for (const key of Object.keys(release).filter((value) => value !== 'binaries')) {
    assert.deepEqual(lab[key], release[key], 'lab may alter only binaries: ' + key);
  }
}
function featureLines(source, name) {
  const line = source.split(/\r?\n/).find((value) => value.startsWith(name + ' = ['));
  assert.ok(line, 'missing exact Cargo feature: ' + name);
  const match = line.match(/^[^=]+ = \[([^\]]*)\]/);
  assert.ok(match, 'cannot parse Cargo feature: ' + name);
  return [...match[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
}
// Follow the actual Cargo feature graph, not only direct installed entries.
// Otherwise an intermediary feature or Cargo default can enable the test mux.
function featureGraph(source) {
  const section = source.split(/\r?\n\[features\]\r?\n/);
  assert.equal(section.length, 2, 'Cargo must have exactly one [features] section');
  const lines = section[1].split(/\r?\n\[/, 1)[0].split(/\r?\n/);
  const graph = new Map();
  for (const line of lines) {
    const match = line.match(/^([a-z][a-z0-9-]*)\s*=\s*\[([^\]]*)\]/);
    if (match) graph.set(match[1], [...match[2].matchAll(/"([^"]+)"/g)].map(x => x[1]));
  }
  return graph;
}
function canReach(graph, start, target, visited = new Set()) {
  if (start === target) return true;
  if (visited.has(start)) return false;
  visited.add(start);
  return (graph.get(start) || []).some(next => canReach(graph, next, target, visited));
}
function validatePackagerRoute(signed, packager) {
  assert.match(signed, /run: \.\/scripts\/akula-win7-signed-stage\.ps1/,
    'HOLD: publisher must build audited Windows binaries');
  assert.match(signed, /run: \.\/scripts\/akula-win7-nsis-candidate\.ps1/,
    'HOLD: publisher must use the reviewed NSIS packager');
  assert.match(signed, /run: \.\/scripts\/test-windows-beta\.ps1/,
    'HOLD: publisher must test a real installed NSIS before signing');
  assert.match(packager, /native\/packaging\/akula-windows\.json/,
    'HOLD: publisher packager must reference the approved two-EXE config');
  assert.doesNotMatch(signed + '\n' + packager,
    /akula-windows-video17-lab\.json/,
    'HOLD: publisher switched to unsigned VIDEO-17 laboratory config');
}

function validateReleaseGates(cargo, signed) {
  const installed = featureLines(cargo, 'installed');
  const ui = featureLines(cargo, 'video-ui-e2e');
  const graph = featureGraph(cargo);
  for (const source of ['default', 'installed']) {
    for (const target of ['video-rust-bundle', 'video-ui-e2e']) {
      assert.equal(canReach(graph, source, target), false,
        `HOLD: Cargo ${source} feature transitively enables unapproved ${target}`);
    }
  }
  assert.ok(!installed.includes('video-rust-bundle') && !installed.includes('video-ui-e2e'),
    'HOLD: shipping installed feature must not run experimental unapproved video toolchain');
  assert.deepEqual(ui, ['video-rust-bundle'], 'GUI test must remain opt-in and require pinned mux');
  assert.match(signed, /if: github\.ref == 'refs\/heads\/main'/);
  assert.match(signed, /environment: akula-release/);
  assert.match(signed, /publish_beta:[\s\S]{0,200}default: false/);
  assert.match(signed, /cargo packager signer sign \$package\[0\]\.FullName/);
  assert.match(signed, /cargo packager signer sign \(Join-Path \$out 'latest\.json'\)/);
  assert.match(signed, /independent-verification:/);
  assert.match(signed, /needs: independent-verification/);
  assert.doesNotMatch(signed, /akula-(?:yt-dlp|quickjs|video-mux)\.exe|video-ui-e2e|video-rust-bundle/,
    'HOLD: publisher workflow cannot silently adopt unapproved helper binary set');
}

// The workflow must re-run whenever any file this security gate inspects
// can change. Otherwise a packager edit can bypass an otherwise green gate.
function validateExperimentalWindowsUpdaterHOLD(releaseSource, applySource) {
  const marker='pub(super) const fn channel_ready() -> bool {';
  const block=releaseSource.split(marker,2)[1]?.split('\n}',1)[0];
  assert.ok(block,'HOLD: missing signed Windows update channel guard');
  assert.match(block,/cfg!\(all\(\s*target_os = "windows",\s*feature = "installed",\s*not\(feature = "video-rust-bundle"\)\s*\)\)/,
    'HOLD: experimental video builds must never inherit normal two-EXE NSIS updates');
  assert.match(applySource,/if !super::release::channel_ready\(\) \{[\s\S]*?return Err\("HOLD:/,
    'HOLD: Windows apply must refuse before staging signed setup and stopping UI');
}
test('VIDEO-18b-12 experimental video build cannot apply ordinary signed two-EXE Windows update',()=>{
  validateExperimentalWindowsUpdaterHOLD(
    file('aquilum-app/native/src/updater/release.rs'),
    file('aquilum-app/native/src/updater/apply.rs')
  );
});
test('VIDEO-18b-12 refuses turning off video-update HOLD or moving it after side effects',()=>{
  const release=file('aquilum-app/native/src/updater/release.rs');
  const apply=file('aquilum-app/native/src/updater/apply.rs');
  const weakened=release.replace('not(feature = "video-rust-bundle")','feature = "video-rust-bundle"');
  assert.notEqual(weakened,release);
  assert.throws(()=>validateExperimentalWindowsUpdaterHOLD(weakened,apply),/HOLD/);
  const bypass=apply.replace('if !super::release::channel_ready() {','if false {');
  assert.notEqual(bypass,apply);
  assert.throws(()=>validateExperimentalWindowsUpdaterHOLD(release,bypass),/HOLD/);
});

function validateHoldTriggerCoverage(workflow) {
  const header = workflow.split(/\r?\npermissions:/, 1)[0];
  assert.match(header, /^on:\r?\n  pull_request:/m, 'HOLD: missing PR event');
  const lines = header.split(/\r?\n/);
  for (const input of [
    '.github/scripts/video18-release-bundle-hold.test.cjs',
    'aquilum-app/native/src/video/light_windows_version.rs',
    'aquilum-app/native/src/updater/release.rs',
    'aquilum-app/native/src/updater/apply.rs',
    'aquilum-app/native/src/updater/tests.rs',
    '.github/workflows/akula-video18-release-bundle-hold.yml',
    '.github/workflows/akula-windows-signed-candidate.yml',
    'aquilum-app/native/Cargo.toml',
    'aquilum-app/native/packaging/akula-windows.json',
    'aquilum-app/native/packaging/akula-windows-video17-lab.json',
    'scripts/akula-win7-nsis-candidate.ps1',
    'scripts/akula-win7-signed-stage.ps1',
    'scripts/test-windows-beta.ps1'
  ]) {
    assert.ok(lines.includes("      - '" + input + "'"),
      'HOLD: missing signed-release trigger ' + input);
  }
}

test('VIDEO-18 public signed NSIS remains two-binary and VIDEO-17 lab is isolated', () => {
  validatePackageSplit(config(RELEASE), config(LAB));
  const signed = file('.github/workflows/akula-windows-signed-candidate.yml');
  validateReleaseGates(file('aquilum-app/native/Cargo.toml'), signed);
  validatePackagerRoute(signed, file('scripts/akula-win7-nsis-candidate.ps1'));
  console.log('VIDEO18_SIGNED_BUNDLE_HOLD_PASS signed_scope=two_exe lab_scope=five_exe release=0');
});

test('VIDEO-18 refuses accidentally promoting a helper into release package', () => {
  const release = config(RELEASE);
  const lab = config(LAB);
  release.binaries.push({path: 'akula-yt-dlp', main: false});
  assert.throws(() => validatePackageSplit(release, lab), /HOLD/);
});

test('VIDEO-18 refuses accidentally enabling experimental GUI mode in installed', () => {
  const cargo = file('aquilum-app/native/Cargo.toml');
  const signed = file('.github/workflows/akula-windows-signed-candidate.yml');
  assert.throws(() => validateReleaseGates(
    cargo.replace(/^installed = \[([^\]]*)\]/m, 'installed = ["video-ui-e2e", $1]'),
    signed
  ), /HOLD/);
});

test('VIDEO-18 refuses signed-candidate workflow without independent verification', () => {
  const cargo = file('aquilum-app/native/Cargo.toml');
  const signed = file('.github/workflows/akula-windows-signed-candidate.yml');
  assert.throws(() => validateReleaseGates(
    cargo,
    signed.replace('needs: independent-verification', 'needs: windows-build')
  ), /independent-verification/);
});

test('VIDEO-18 refuses Cargo-default activation of test-only video code', () => {
  const cargo = file('aquilum-app/native/Cargo.toml');
  const signed = file('.github/workflows/akula-windows-signed-candidate.yml');
  assert.throws(
    () => validateReleaseGates(cargo.replace('[features]', '[features]\ndefault = ["video-rust-bundle"]'), signed),
    /HOLD/
  );
});

test('VIDEO-18 refuses a transitive activation via a normal installed feature', () => {
  const cargo = file('aquilum-app/native/Cargo.toml');
  const signed = file('.github/workflows/akula-windows-signed-candidate.yml');
  const bad = cargo.replace(
    'server-readonly-preflight = ["aquilum-core/server-bootstrap-b0-openssh-lab"]',
    'server-readonly-preflight = ["aquilum-core/server-bootstrap-b0-openssh-lab", "video-rust-bundle"]'
  );
  assert.notEqual(bad, cargo, 'the mutation must alter an existing Cargo feature');
  assert.throws(() => validateReleaseGates(bad, signed), /HOLD/);
});

test('VIDEO-18 refuses switching the signed publisher to the unsigned lab packager', () => {
  const signed = file('.github/workflows/akula-windows-signed-candidate.yml');
  const packager = file('scripts/akula-win7-nsis-candidate.ps1');
  const changed = packager.replace(
    'native/packaging/akula-windows.json',
    'native/packaging/akula-windows-video17-lab.json'
  );
  assert.notEqual(changed, packager, 'the mutation must change the reviewed packaging route');
  assert.throws(() => validatePackagerRoute(signed, changed), /HOLD/);
});

test('VIDEO-18 cannot silently lose signed-installer workflow triggers', () => {
  const workflow = file('.github/workflows/akula-video18-release-bundle-hold.yml');
  validateHoldTriggerCoverage(workflow);
  const removed = workflow.replace("      - 'scripts/akula-win7-nsis-candidate.ps1'\n", '');
  assert.notEqual(removed, workflow, 'the negative test must remove a real path');
  assert.throws(() => validateHoldTriggerCoverage(removed), /HOLD: missing signed-release trigger/);
});

function validateWindowsVideoRuntimeGate(light, guard) {
  assert.match(light,
    /#\[cfg\(all\(windows, feature = "video-rust-bundle"\)\)\]\s*#\[path = "light_windows_version\.rs"\]\s*mod windows_version;/,
    'HOLD: Windows runtime gate module must be feature-scoped');
  const route = light.split('pub(super) fn download_one_or_legacy(', 2)[1];
  assert.ok(route, 'HOLD: missing bundled downloader');
  const gate = route.indexOf('windows_version::require_pinned_windows_video_runtime()?;');
  const locate = route.indexOf('let application = std::env::current_exe()');
  assert.ok(gate >= 0 && locate > gate,
    'HOLD: pinned runtime must reject unsupported OS before locating tools');
  assert.match(guard, /fn RtlGetVersion\(version: \*mut OsVersionInfo\) -> i32;/,
    'HOLD: Windows version must be queried from ntdll');
  assert.match(guard, /major\s*>=\s*10\b/,
    'HOLD: pinned video runtime must require Windows 10 or newer');
  assert.match(guard, /if unsafe \{ RtlGetVersion\(&mut version\) \} != 0/,
    'HOLD: OS-probe failure must prevent executing helpers');
  assert.match(guard, /!supports_pinned_windows_video_runtime\(version\.major\)/,
    'HOLD: unsupported OS must be rejected at runtime');
}

test('VIDEO-18b experimental video downloader has a pre-spawn Windows 10+ gate', () => {
  validateWindowsVideoRuntimeGate(
    file('aquilum-app/native/src/video/light.rs'),
    file('aquilum-app/native/src/video/light_windows_version.rs')
  );
});

test('VIDEO-18b refuses removing the OS gate or lowering the Windows version floor', () => {
  const light = file('aquilum-app/native/src/video/light.rs');
  const guard = file('aquilum-app/native/src/video/light_windows_version.rs');
  const noCall = light.replace('windows_version::require_pinned_windows_video_runtime()?;', '');
  assert.notEqual(noCall, light);
  assert.throws(() => validateWindowsVideoRuntimeGate(noCall, guard), /HOLD/);
  const win7 = guard.replace('major >= 10', 'major >= 6');
  assert.notEqual(win7, guard);
  assert.throws(() => validateWindowsVideoRuntimeGate(light, win7), /HOLD/);
  const unsupportedProbe = guard.replace('fn RtlGetVersion(', 'fn GetVersionExW(');
  assert.notEqual(unsupportedProbe, guard);
  assert.throws(() => validateWindowsVideoRuntimeGate(light, unsupportedProbe), /HOLD/);
});

function validateVersionGuardWorkflowCoverage(contents) {
  const source = 'aquilum-app/native/src/video/light_windows_version.rs';
  for (const [name, body] of Object.entries(contents)) {
    const header = body.split(/\r?\npermissions:/, 1)[0];
    assert.match(header, /^on:\r?\n  pull_request:/m, 'HOLD: missing PR trigger ' + name);
    assert.ok(header.split(/\r?\n/).includes("      - '" + source + "'"),
      'HOLD: missing Windows-version source trigger for ' + name);
  }
}

test('VIDEO-18b changes to Windows-version guard must rerun real native GUI/NSIS and HOLD', () => {
  const names = [
    'akula-video13-trusted-mux-candidate.yml',
    'akula-video16-real-windows-gui.yml',
    'akula-video17-nsis-installed-video.yml',
    'akula-video18-release-bundle-hold.yml'
  ];
  const workflows = Object.fromEntries(names.map(name =>
    [name, file('.github/workflows/' + name)]));
  validateVersionGuardWorkflowCoverage(workflows);
  const name = names[2];
  const mut = {...workflows, [name]: workflows[name].replace(
    "      - 'aquilum-app/native/src/video/light_windows_version.rs'\n", '')};
  assert.notEqual(mut[name], workflows[name], 'negative trigger mutation must modify workflow');
  assert.throws(() => validateVersionGuardWorkflowCoverage(mut),
    /HOLD: missing Windows-version source trigger/);
});
