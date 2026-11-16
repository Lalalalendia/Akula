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
    () => validateReleaseGates(cargo.replace('[features]', '[features]\\ndefault = ["video-rust-bundle"]'), signed),
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
