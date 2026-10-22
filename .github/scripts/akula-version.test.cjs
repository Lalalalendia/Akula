'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { load, parts, compare, check, planBump, nextPatch } = require('../../scripts/akula-version.cjs');

const files = load();
const current = check(files);

test('Windows release version is identical in both Cargo manifests, lock and NSIS config', () => {
  assert.match(current, /^\d+\.\d+\.\d+$/);
  assert.equal(check(files), current);
});

test('single bump changes only four Windows version files and stays locked', () => {
  const next = nextPatch(current);
  const planned = planBump(files, next);
  assert.equal(check(planned), next);
  for (const name of ['native', 'core', 'lock', 'windows']) {
    assert.notEqual(planned[name], files[name], name);
  }
  assert.equal(planned.linux, files.linux, 'Linux HOLD is a separate release track');
});

test('refuse equal, older, malformed and ambiguous versions', () => {
  assert.throws(() => planBump(files, current), /strictly exceed/);
  assert.throws(() => planBump(files, '0.0.0'), /strictly exceed/);
  for (const bad of ['v0.3.11', '01.2.3', '1.2', '1.2.3-rc', '1.2.3.4', 'a.b.c']) {
    assert.throws(() => parts(bad), /three-part/);
  }
  assert.equal(compare('0.3.11', '0.3.9'), 1, 'numeric, not lexical ordering');
});

test('fail closed on every version mirror mismatch or unexpected package identity', () => {
  const fixtures = [
    { ...files, native: files.native.replace('name = "aquilum-native"', 'name = "other-native"') },
    { ...files, core: files.core.replace('name = "aquilum-core"', 'name = "other-core"') },
    { ...files, lock: files.lock.replace('name = "aquilum-core"', 'name = "other-core"') },
    { ...files, lock: files.lock.replace('name = "aquilum-native"', 'name = "other-native"') },
    { ...files, windows: files.windows.replace('"productName": "Akula"', '"productName": "Aquilum"') },
    { ...files, windows: files.windows.replace('"version": "' + current + '"', '"version": "0.0.1"') },
  ];
  for (const fixture of fixtures) assert.throws(() => check(fixture));
});

test('Linux publisher version is separate and must be explicitly synced for a Linux release', () => {
  const linux = JSON.parse(files.linux);
  assert.equal(linux.productName, 'Akula');
  if (linux.version !== current) {
    assert.throws(() => check(files, true), /Linux packager/);
  } else {
    assert.equal(check(files, true), current);
  }
});
