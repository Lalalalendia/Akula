'use strict';
// RELEASE-TRUST-LAB-01: offline, deny-only candidate verifier.
// This laboratory never signs for Akula, installs packages or enables updates.
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const cp = require('node:child_process');
const {TextDecoder} = require('node:util');

const PLATFORMS = ['windows-x86_64', 'linux-x86_64-appimage', 'linux-x86_64-deb', 'linux-x86_64-rpm'];
const SHA = /^[a-f0-9]{64}$/;
const COMMIT = /^[a-f0-9]{40}$/;
const MANIFEST_LIMIT = 65536;
const ARTIFACT_LIMIT = 1024 * 1024 * 1024;
const CHUNK = 64 * 1024;

function demand(ok, message) { if (!ok) throw new Error(message); }
function object(value) { return value !== null && typeof value === 'object' && !Array.isArray(value); }
function exactKeys(o, keys, name) {
  demand(object(o), name + ' must be an object');
  demand(JSON.stringify(Object.keys(o).sort()) === JSON.stringify([...keys].sort()),
    name + ' has missing or unknown fields');
}
function openRegular(file) {
  // Never silently fall back to following an attacker-supplied leaf symlink.
  demand(typeof fs.constants.O_NOFOLLOW === 'number', 'platform cannot enforce no-follow');
  const fd = fs.openSync(file, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  const stat = fs.fstatSync(fd);
  if (!stat.isFile()) { fs.closeSync(fd); throw new Error('not a regular file: ' + file); }
  return {fd, stat};
}
function sha256File(file, expectedSize) {
  const {fd,stat} = openRegular(file);
  try {
    demand(stat.size === expectedSize, 'file type/size mismatch: ' + path.basename(file));
    const hash = crypto.createHash('sha256');
    const block = Buffer.allocUnsafe(CHUNK);
    let count = 0, read = 0;
    while ((read = fs.readSync(fd, block, 0, block.length, null)) !== 0) {
      count += read;
      demand(count <= expectedSize, 'artifact grew during verification');
      hash.update(block.subarray(0, read));
    }
    const end = fs.fstatSync(fd);
    demand(count === expectedSize && end.size === expectedSize &&
      end.dev === stat.dev && end.ino === stat.ino,
      'artifact changed while checking: ' + path.basename(file));
    return hash.digest('hex');
  } finally {
    fs.closeSync(fd);
  }
}
function boundedManifest(file) {
  const {fd,stat} = openRegular(file);
  try {
    demand(stat.size > 0 && stat.size <= MANIFEST_LIMIT, 'manifest size invalid');
    const parts = [], block = Buffer.allocUnsafe(CHUNK);
    let total = 0, read = 0;
    while ((read = fs.readSync(fd, block, 0, block.length, null)) !== 0) {
      total += read;
      demand(total <= MANIFEST_LIMIT, 'manifest size invalid');
      parts.push(Buffer.from(block.subarray(0, read)));
    }
    demand(total !== 0, 'manifest size invalid');
    return Buffer.concat(parts, total);
  } finally {
    fs.closeSync(fd);
  }
}
function properName(platform, name, version) {
  const v = version.replace(/\./g, '\\.');
  const patterns = {
    'windows-x86_64': '^Akula_' + v + '_[A-Za-z0-9-]+-setup\\.exe$',
    'linux-x86_64-appimage': '^akula-app_' + v + '_x86_64\\.AppImage$',
    'linux-x86_64-deb': '^Akula_' + v + '_amd64\\.deb$',
    'linux-x86_64-rpm': '^Akula-' + v + '-1\\.x86_64\\.rpm$',
  };
  return new RegExp(patterns[platform]).test(name);
}
function inspect(manifest, directory, expected) {
  exactKeys(manifest, ['schema','product','version','source_commit','cargo_lock_sha256','artifacts'], 'manifest');
  demand(manifest.schema === 1 && manifest.product === 'Akula', 'wrong product/schema');
  demand(typeof manifest.version === 'string' &&
    /^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$/.test(manifest.version), 'invalid version');
  demand(typeof manifest.source_commit === 'string' && COMMIT.test(manifest.source_commit), 'invalid source SHA');
  demand(typeof manifest.cargo_lock_sha256 === 'string' && SHA.test(manifest.cargo_lock_sha256), 'invalid Cargo.lock SHA');
  demand(manifest.version === expected.version &&
    manifest.source_commit === expected.source_commit &&
    manifest.cargo_lock_sha256 === expected.cargo_lock_sha256, 'unexpected release identity');
  exactKeys(manifest.artifacts, PLATFORMS, 'platforms');

  // Validate all names before reading any artifact: error order is deterministic.
  const seen = new Set(), entries = [];
  for (const platform of PLATFORMS) {
    const item = manifest.artifacts[platform];
    exactKeys(item, ['filename','sha256','size'], platform);
    const name = item.filename;
    demand(typeof name === 'string' &&
      /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(name) && name !== '.' && name !== '..',
      'unsafe artifact filename');
    demand(!seen.has(name), 'duplicate artifact filename');
    seen.add(name);
    demand(typeof item.sha256 === 'string' && SHA.test(item.sha256), 'invalid artifact digest');
    demand(Number.isSafeInteger(item.size) && item.size > 0 && item.size <= ARTIFACT_LIMIT,
      'invalid artifact size');
    entries.push({platform, name, item});
  }
  for (const {platform, name, item} of entries) {
    demand(properName(platform, name, manifest.version), 'platform filename mismatch: ' + platform);
    const digest = sha256File(path.join(directory, name), item.size);
    demand(digest === item.sha256, 'artifact digest mismatch: ' + name);
  }
  return true;
}
function decodeManifest(raw) {
  // Reject ambiguous signed JSON (including duplicate keys or trailing data).
  const text = new TextDecoder('utf-8', {fatal:true}).decode(raw);
  const value = JSON.parse(text);
  demand(Buffer.from(JSON.stringify(value), 'utf8').equals(raw), 'noncanonical manifest JSON');
  return value;
}
function main(argv) {
  demand(argv.length === 7,
    'usage: node verify-release-candidate.cjs manifest.json manifest.minisig trusted.pub artifacts-dir version source-commit cargo-lock-sha256');
  const [manifestFile, signatureFile, keyFile, dir, version, source_commit, cargo_lock_sha256] = argv;
  // Minisign reads a *snapshot*, never a path that can change after parsing.
  const raw = boundedManifest(manifestFile);
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'akula-trust-snapshot-'));
  try {
    const snapshot = path.join(work, 'manifest.json');
    fs.writeFileSync(snapshot, raw, {mode:0o600, flag:'wx'});
    const result = cp.spawnSync('minisign',
      ['-Vm', snapshot, '-x', signatureFile, '-p', keyFile, '-Q'],
      {stdio:'pipe', encoding:'utf8', timeout:15000});
    demand(!result.error && result.status === 0, 'publisher signature verification failed');
    inspect(decodeManifest(raw), dir, {version, source_commit, cargo_lock_sha256});
    console.log('AKULA_TRUST_LAB_OFFLINE_PASS');
  } finally {
    fs.rmSync(work, {recursive:true, force:true});
  }
}
if (require.main === module) {
  try { main(process.argv.slice(2)); }
  catch (error) {
    console.error('AKULA_TRUST_LAB_HOLD:', error.message);
    process.exitCode = 1;
  }
}
module.exports = {inspect, main, PLATFORMS, decodeManifest, boundedManifest};
