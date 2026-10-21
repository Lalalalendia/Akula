'use strict';

// RELEASE-LOCK-01 — fail closed if someone re-enables the inherited Aquilum
// release job or removes its guard before the independent Akula release exists.
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');

const root = path.resolve(__dirname, '../..');
const file = (name) => readFileSync(path.join(root, name), 'utf8');

test('release.yml is a manually-triggered, read-only HOLD with one failing job', () => {
  const text = file('.github/workflows/release.yml');
  assert.match(text, /^name: Akula Release \(HOLD\)$/m);
  assert.match(text, /^on:\s*\n  workflow_dispatch:\s*$/m);
  assert.match(text, /^permissions:\s*\n  contents: read$/m);
  const jobs = text.split(/\njobs:\n/);
  assert.equal(jobs.length, 2, 'release.yml must contain one jobs stanza');
  const names = [...jobs[1].matchAll(/^  ([a-z][a-z0-9-]*):\s*$/gm)].map((match) => match[1]);
  assert.deepEqual(names, ['release-hold'], 'no executable packaging/publishing jobs');
  assert.match(jobs[1], /echo "::error title=Akula release HOLD::/);
  assert.match(jobs[1], /^\s+exit 1\s*$/m, 'manual invocation must always fail');
  assert.doesNotMatch(text, /Aquilum_0\.2\.6|cargo packager|gh release|TAURI_SIGNING|upload-artifact|secrets\./);
});

test('inherited workflow is preserved only as documentation', () => {
  const archive = file('docs/release/legacy-aquilum-release.yml');
  assert.match(archive, /gh release download v0\.2\.6 -R Freaction\/Aquilum/);
  assert.match(archive, /^  macos:/m);
  assert.match(archive, /^  linux:/m);
  assert.match(archive, /^  manifest:/m);
  assert.doesNotMatch(file('.github/workflows/release.yml'), /Aquilum_0\.2\.6/);
});

test('safe standalone Windows beta remains separate; updater remains off', () => {
  const beta = file('.github/workflows/windows-beta.yml');
  const updater = file('aquilum-app/native/src/updater/release.rs');
  assert.match(beta, /^name: Akula Windows beta candidate$/m);
  assert.match(beta, /--features installed/);
  assert.match(beta, /scripts\/test-windows-beta\.ps1/);
  assert.doesNotMatch(beta, /sync-installed-host/);
  const pinned = updater.match(/pub\(super\) const PUBLIC_KEY: &str = "([^"]+)";/);
  assert.ok(pinned, 'Akula must have an explicit pinned public key');
  // Exact key bytes, not merely the human-readable comment, are trust-critical.
  assert.equal(pinned[1], 'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEQ3QTY2QjZENDA0NkY2QzUKUldURjlrWkFiV3VtMXh2SFFsdEljaUw0T0pUakVmd1krZG4wVmJ1REVmaEx1eWcrMFIzTjQzMi8K', 'Akula publisher root changed without a reviewed key rotation');
  const decoded = Buffer.from(pinned[1], 'base64').toString('utf8');
  assert.equal(decoded.split('\n')[0], 'untrusted comment: minisign public key: D7A66B6D4046F6C5');
  assert.match(decoded.split('\n')[1], /^RW[A-Za-z0-9+/]+$/);
  assert.equal(decoded.split('\n').filter(Boolean).length, 2);
  assert.match(updater, /pub\(super\) const fn channel_ready\(\) -> bool \{\s*false\s*\}/);
});

test('this policy runs when the release workflow is edited', () => {
  const ci = file('.github/workflows/release-isolation-policy.yml');
  assert.match(ci, /^name: Akula release isolation policy$/m);
  assert.match(ci, /^  pull_request:$/m);
  assert.match(ci, /\.github\/workflows\/release\.yml/);
  assert.match(ci, /node --test \.github\/scripts\/release-isolation-policy\.test\.cjs/);
});


test('publisher key proof cannot publish, run on PR code or bypass protected environment', () => {
  const yaml = file('.github/workflows/akula-publisher-key-proof.yml');
  assert.match(yaml, /^name: Akula publisher key proof \(no release\)$/m);
  assert.match(yaml, /^on:\n  workflow_dispatch:\s*$/m);
  assert.match(yaml, /^permissions:\n  contents: read$/m);
  assert.match(yaml, /if: github.ref == 'refs\/heads\/main'/);
  assert.match(yaml, /environment: akula-release/);
  assert.match(yaml, /secrets\.CARGO_PACKAGER_SIGN_PRIVATE_KEY/);
  assert.match(yaml, /secrets\.CARGO_PACKAGER_SIGN_PRIVATE_KEY_PASSWORD/);
  assert.match(yaml, /minisign -Vm /);
  assert.match(yaml, /tamper_refused=1 no_release=1/);
  assert.doesNotMatch(yaml, /gh release|create-release|contents:\s*write|upload-artifact|pull_request:|push:/);
});


test('signed Windows candidate uses protected main-only secrets and cannot publish a Release', () => {
  const yaml = file('.github/workflows/akula-windows-signed-candidate.yml');
  assert.match(yaml, /^name: Akula Windows signed candidate \(review only\)$/m);
  assert.match(yaml, /^on:\n  workflow_dispatch:\s*$/m);
  assert.match(yaml, /^permissions:\n  contents: read$/m);
  assert.match(yaml, /if: github.ref == 'refs\/heads\/main'/);
  assert.match(yaml, /environment: akula-release/);
  assert.match(yaml, /secrets\.CARGO_PACKAGER_SIGN_PRIVATE_KEY/);
  assert.match(yaml, /secrets\.CARGO_PACKAGER_SIGN_PRIVATE_KEY_PASSWORD/);
  assert.match(yaml, /test-windows-beta\.ps1/);
  assert.match(yaml, /signed_manifest=1 independent_verify=1 tamper_refused=1 no_release=1/);
  assert.doesNotMatch(yaml, /gh release|create-release|contents:\s*write|pull_request:|push:/);
});
