'use strict';

// PLUGIN-00 checks documentation integrity, not guest-runtime security.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.resolve(__dirname, '..', '..');
const docs = path.join(root, 'docs', 'plugins');
const inventory = JSON.parse(fs.readFileSync(path.join(docs, 'architecture-inventory.json'), 'utf8'));
const read = name => fs.readFileSync(path.join(docs, name), 'utf8');

function unique(items) {
  return new Set(items).size === items.length;
}

test('PLUGIN-00 inventory is explicitly design-only and pinned to a commit', () => {
  assert.equal(inventory.schema, 'akula.plugin-architecture/0');
  assert.equal(inventory.status, 'design-only');
  assert.match(inventory.sourceCommit, /^[0-9a-f]{40}$/);
  assert.ok(inventory.invariants.length >= 9);
  assert.ok(unique(inventory.invariants.map(x => x.id)));
  assert.ok(inventory.invariants.every(x => /^INV-\d\d$/.test(x.id) && /^PLUGIN-\d\d$/.test(x.firstGate)));
  assert.ok(inventory.separatePrograms.some(x => x.id === 'SYNC' && /built-in/.test(x.implementation)));
});

test('documented host boundaries map to real tracked source paths and symbols', () => {
  assert.ok(inventory.surfaces.length >= 9);
  assert.ok(unique(inventory.surfaces.map(x => x.id)));
  for (const surface of inventory.surfaces) {
    assert.match(surface.path, /^(?:aquilum-app|\.github)\//);
    assert.equal(path.posix.normalize(surface.path), surface.path);
    const file = path.join(root, surface.path);
    assert.ok(fs.existsSync(file), 'missing source ' + surface.path);
    const source = fs.readFileSync(file, 'utf8');
    for (const symbol of surface.expectedSymbols) {
      assert.ok(source.includes(symbol), surface.path + ' lacks symbol ' + symbol);
    }
    assert.ok(surface.boundary.length > 10);
  }
});

test('threat model and inventory map all 16 distinct falsifiers to later gates', () => {
  const ids = inventory.threats.map(x => x.id);
  assert.deepEqual(ids, Array.from({length:16},(_,i) => 'T' + String(i+1).padStart(2,'0')));
  assert.ok(unique(ids));
  for (const threat of inventory.threats) {
    assert.match(threat.firstGate, /^PLUGIN-\d\d$/);
    assert.ok(threat.scenario.length > 8 && threat.falsifier.length > 8);
  }
  const inMarkdown = [...read('threat-model.md').matchAll(/^### (T\d\d) \u2014/gm)].map(x => x[1]);
  assert.deepEqual(inMarkdown, ids, 'threat headings must match the machine inventory');
});

test('architectural docs link the same canonical program and disallow ambient guest rights', () => {
  const adr = read('adr-0001-trust-boundaries.md');
  const intro = read('README.md');
  const core = read('core-inventory.md');
  const baseline = read('baseline.md');
  assert.match(adr, /Deny by default/i);
  assert.match(adr, /DocumentHub/);
  assert.match(adr, /File Gate/);
  assert.match(core, /not a dynamically loaded plugin/i);
  assert.match(intro, /PLUGIN-00/);
  assert.match(baseline, /Not measured here/);
  assert.ok(['filesystem','network','process','environment','ffi'].every(x => inventory.forbiddenGuestImports.includes(x)));
});
