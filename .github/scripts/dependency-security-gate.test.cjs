'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { mustScanLocally, verdict, MAX_AGE_MS } = require('./dependency-security-gate.cjs');

const now = Date.UTC(2026, 9, 22, 12);
const run = (conclusion = 'success', status = 'completed') => ({
  id: 41, event: 'schedule', head_branch: 'main', status, conclusion,
  updated_at: new Date(now - 1000).toISOString(),
});
const job = (conclusion = 'success', ageMs = 60000) => ({
  name: 'RustSec dependency audit', conclusion,
  completed_at: new Date(now - ageMs).toISOString(),
});
test('recent real RustSec success admits, not a skipped decision', () => {
  assert.equal(verdict(run(), [job()], now).kind, 'allow');
  assert.equal(verdict(run(), [job('skipped')], now).kind, 'skip');
});
test('watchdog failure never falls back to an older success', () => {
  assert.equal(verdict(run('failure'), [job('failure')], now).kind, 'deny');
  assert.equal(verdict(run('failure'), [job('skipped')], now).kind, 'deny');
});
test('missing, queued, stale and future dated audits are HOLD', () => {
  assert.equal(verdict(run(), [], now).kind, 'deny');
  assert.equal(verdict(run(null, 'in_progress'), [job(null)], now).kind, 'deny');
  assert.equal(verdict(run(), [job('success', MAX_AGE_MS + 1000)], now).kind, 'deny');
  assert.equal(verdict(run(), [job('success', -10 * 60 * 1000)], now).kind, 'deny');
});
test('only expected production dependencies and security policy require PR rescans', () => {
  assert.equal(mustScanLocally(['docs/README.md']), false);
  assert.equal(mustScanLocally(['aquilum-app/Cargo.lock']), true);
  assert.equal(mustScanLocally(['aquilum-app/host-broker-lab/Cargo.toml']), true);
  assert.equal(mustScanLocally(['.github/workflows/app-checks.yml']), true);
  assert.equal(mustScanLocally(['.github/scripts/dependency-security-gate.cjs']), true);
});

test('one-time security bootstrap allowlist cannot absorb application changes', () => {
  const { BOOTSTRAP_FILES } = require('./dependency-security-gate.cjs');
  assert.deepEqual([...BOOTSTRAP_FILES].sort(), [
    '.github/scripts/dependency-security-gate.cjs',
    '.github/scripts/dependency-security-gate.test.cjs',
    '.github/workflows/app-checks.yml',
    '.github/workflows/dependency-security.yml',
  ].sort());
  assert.equal(BOOTSTRAP_FILES.has('aquilum-app/Cargo.lock'), false);
  assert.equal(BOOTSTRAP_FILES.has('aquilum-app/native/src/main.rs'), false);
});
