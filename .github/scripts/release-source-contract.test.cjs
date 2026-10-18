'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { validateRelease } = require('./release-source-contract.cjs');

const original = fs.readFileSync(path.resolve(__dirname, '..', 'workflows', 'release.yml'), 'utf8');

function mustReject(yaml, reason) {
  const errors = validateRelease(yaml);
  assert.ok(errors.length, 'unsafe release contract unexpectedly accepted: ' + reason);
  assert.ok(errors.some(error => error.includes(reason)), JSON.stringify(errors));
}

test('pinned multi-platform local-only release contract is valid', () => {
  assert.deepEqual(validateRelease(original), []);
});

test('moving branch ref substituted in a platform checkout is rejected', () => {
  const bad = original.replace(
    'ref: ${{ needs.source.outputs.sha }}',
    'ref: ${{ inputs.ref }}',
  );
  mustReject(bad, 'does not checkout');
});

test('release must not accidentally enable custody without an OS broker', () => {
  const bad = original.replace(
    'cargo build --release -p aquilum-native --features installed',
    'cargo build --release -p aquilum-native --features installed,sync-installed-host',
  );
  mustReject(bad, 'production build');
});

test('releasing arbitrary optional features is rejected', () => {
  const bad = original.replace(
    'cargo build --release -p aquilum-native --features installed',
    'cargo build --release -p aquilum-native --all-features',
  );
  mustReject(bad, 'production build');
});

test('manifest must depend on same immutable source', () => {
  const bad = original.replace(
    'needs: [source, windows, macos, linux]',
    'needs: [windows, macos, linux]',
  );
  mustReject(bad, 'does not depend');
});

test('manifest cannot falsely describe an installed-host build as local-only', () => {
  const bad = original.replace('custody_mode: "local-only"', 'custody_mode: "installer-protected"');
  mustReject(bad, 'manifest missing');
});

test('runtime-supplied host DB cannot slip into local-only release', () => {
  const bad = original.replace('name: Release', 'name: Release\n# AKULA_INSTALLED_HOST_CUSTODY_DB=untrusted');
  mustReject(bad, 'Unsafe custody/S3 feature');
});
