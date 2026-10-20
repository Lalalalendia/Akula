'use strict';
// RELEASE-TRUST-LAB-01: offline, deny-only release candidate verifier.
// This laboratory never publishes releases or enables the updater.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const cp = require('node:child_process');

const PLATFORMS = ['windows-x86_64', 'linux-x86_64-appimage', 'linux-x86_64-deb', 'linux-x86_64-rpm'];
const SHA = /^[a-f0-9]{64}$/;
const COMMIT = /^[a-f0-9]{40}$/;
function demand(ok, message) { if (!ok) throw new Error(message); }
function object(value) { return value !== null && typeof value === 'object' && !Array.isArray(value); }
function exactKeys(o, keys, name) {
  demand(object(o), name + ' must be an object');
  demand(JSON.stringify(Object.keys(o).sort()) === JSON.stringify([...keys].sort()), name + ' has missing or unknown fields');
}
function inspect(manifest, directory, expected) {
  exactKeys(manifest, ['schema','product','version','source_commit','cargo_lock_sha256','artifacts'], 'manifest');
  demand(manifest.schema === 1 && manifest.product === 'Akula', 'wrong product/schema');
  demand(typeof manifest.version === 'string' && /^([0-9]+)\.([0-9]+)\.([0-9]+)(-[a-zA-Z0-9.-]+)?$/.test(manifest.version), 'invalid version');
  demand(COMMIT.test(manifest.source_commit), 'invalid source SHA');
  demand(SHA.test(manifest.cargo_lock_sha256), 'invalid Cargo.lock SHA');
  demand(manifest.version === expected.version && manifest.source_commit === expected.source_commit && manifest.cargo_lock_sha256 === expected.cargo_lock_sha256, 'unexpected release identity');
  exactKeys(manifest.artifacts, PLATFORMS, 'platforms');
  const names = new Set();
  for (const platform of PLATFORMS) {
    const item = manifest.artifacts[platform];
    exactKeys(item, ['filename','sha256','size'], platform);
    const name = item.filename;
    demand(typeof name === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(name) && name !== '.' && name !== '..', 'unsafe artifact filename');
    demand(!names.has(name), 'duplicate artifact filename');
    names.add(name);
    demand(SHA.test(item.sha256), 'invalid artifact digest');
    demand(Number.isSafeInteger(item.size) && item.size > 0 && item.size <= 1024 * 1024 * 1024, 'invalid artifact size');
    const full = path.join(directory, name);
    const stat = fs.lstatSync(full);
    demand(stat.isFile() && !stat.isSymbolicLink() && stat.size === item.size, 'file type/size mismatch: ' + name);
    const bytes = fs.readFileSync(full);
    demand(crypto.createHash('sha256').update(bytes).digest('hex') === item.sha256, 'artifact digest mismatch: ' + name);
  }
  return true;
}
function main(argv) {
  demand(argv.length === 7, 'usage: node verify-release-candidate.cjs manifest.json manifest.minisig trusted.pub artifacts-dir version source-commit cargo-lock-sha256');
  const [manifestFile, signatureFile, keyFile, dir, version, source_commit, cargo_lock_sha256] = argv;
  const raw = fs.readFileSync(manifestFile);
  demand(raw.length > 0 && raw.length <= 65536, 'manifest size invalid');
  const result = cp.spawnSync('minisign', ['-Vm', manifestFile, '-x', signatureFile, '-p', keyFile, '-Q'], {stdio:'pipe', encoding:'utf8', timeout:15000});
  demand(!result.error && result.status === 0, 'publisher signature verification failed');
  inspect(JSON.parse(raw.toString('utf8')), dir, {version, source_commit, cargo_lock_sha256});
  console.log('AKULA_TRUST_LAB_OFFLINE_PASS');
}
if (require.main === module) {
  try { main(process.argv.slice(2)); } catch (error) {
    console.error('AKULA_TRUST_LAB_HOLD:', error.message);
    process.exitCode = 1;
  }
}
module.exports = {inspect, main, PLATFORMS};
