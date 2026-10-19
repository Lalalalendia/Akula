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


test('manifest must not redirect new Akula downloads to the upstream project', () => {
  const bad = original.replace(
    'url="https://github.com/Lalalalendia/Akula/releases/download/v$version"',
    'url="https://github.com/Freaction/Aquilum/releases/download/v$version"',
  );
  mustReject(bad, 'Release manifest must publish artifacts');
});

test('missing Akula verification key must fail closed', () => {
  const bad = original.replace('test -n "$key" || {', 'true # key not checked');
  mustReject(bad, 'Release must stop when');
});


test('macOS release artifact cannot skip publisher and notarization verification', () => {
  const bad = original.replace(
    '- name: Verify pinned Akula publisher and notarized DMG',
    '- name: Bypass publisher verification',
  );
  mustReject(bad, 'macos must verify genuine signed DMG');
});

test('macOS release verifier cannot consume an arbitrary source policy', () => {
  const bad = original.replace(
    'bash scripts/verify-macos-release.sh scripts/install.sh "${dmgs[0]}" "${archives[0]}"',
    'bash scripts/verify-macos-release.sh /tmp/injected-install.sh "${dmgs[0]}"',
  );
  mustReject(bad, 'macos release verifier must use exact pinned source and DMG');
});

test('macOS release verifier must check exactly one packaged DMG', () => {
  const bad = original.replace(
    'if [[ "${#dmgs[@]}" -ne 1 ]]',
    'if [[ "${#dmgs[@]}" -lt 1 ]]',
  );
  mustReject(bad, 'macos release verifier must use exact pinned source and DMG');
});

test('macOS release verifier must not run after artifact upload', () => {
  const before = '      - name: Verify pinned Akula publisher and notarized DMG';
  const upload = '      - uses: actions/upload-artifact@v7';
  const start = original.indexOf(before, original.indexOf('  macos:'));
  const finish = original.indexOf('      - name: Install from DMG', start);
  const verifyBlock = original.slice(start, finish);
  const without = original.slice(0, start) + original.slice(finish);
  const uploadAt = without.indexOf(upload, without.indexOf('  macos:'));
  assert.ok(start >= 0 && finish > start && uploadAt > start);
  const bad = without.slice(0, uploadAt) + verifyBlock + without.slice(uploadAt);
  // Verify block before upload is acceptable; move it after upload instead.
  const after = bad.replace(verifyBlock, '');
  const uploaded = after.indexOf(upload, after.indexOf('  macos:'));
  const next = after.indexOf('      - uses: actions/upload-artifact@v7', uploaded + upload.length);
  assert.ok(next > uploaded);
  const unsafe = after.slice(0, next) + verifyBlock + after.slice(next);
  mustReject(unsafe, 'macos must verify genuine signed DMG');
});

test('macOS updater archive parity cannot be omitted from notarized DMG release', () => {
  const bad = original.replace(
    'bash scripts/verify-macos-release.sh scripts/install.sh "${dmgs[0]}" "${archives[0]}"',
    'bash scripts/verify-macos-release.sh scripts/install.sh "${dmgs[0]}"',
  );
  mustReject(bad, 'macos release verifier must use exact pinned source and DMG');
});
