'use strict';

// One Windows beta version: native Cargo is canonical; three mirrors must agree.
// Linux packaging stays on its separately gated, unpublished version.
const fs = require('node:fs');
const path = require('node:path');
const ROOT = path.resolve(__dirname, '..');
const PATHS = Object.freeze({
  native: 'aquilum-app/native/Cargo.toml',
  core: 'aquilum-app/core/Cargo.toml',
  lock: 'aquilum-app/Cargo.lock',
  windows: 'aquilum-app/native/packaging/akula-windows.json',
  linux: 'aquilum-app/native/packaging/akula-linux.json',
});

function parts(version) {
  if (typeof version !== 'string' ||
      !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) {
    throw Error('expected exact three-part numeric Akula version');
  }
  const value = version.split('.').map(Number);
  if (!value.every(Number.isSafeInteger)) throw Error('Akula version component overflow');
  return value;
}

function compare(a, b) {
  const aa = parts(a), bb = parts(b);
  for (let i = 0; i < 3; i++) {
    if (aa[i] !== bb[i]) return aa[i] > bb[i] ? 1 : -1;
  }
  return 0;
}

function load(root = ROOT) {
  return Object.fromEntries(Object.entries(PATHS).map(([key, file]) =>
    [key, fs.readFileSync(path.join(root, file), 'utf8')]));
}

function packageMarker(kind, name) {
  return (kind === 'lock' ? '[[package]]' : '[package]') +
    '\nname = "' + name + '"\nversion = "';
}

function pkgVersion(text, kind, name) {
  const marker = packageMarker(kind, name);
  const hits = text.split(marker);
  if (hits.length !== 2) throw Error('expected exactly one ' + name + ' in ' + kind);
  const value = hits[1].split('"', 1)[0];
  parts(value);
  return value;
}

function check(files, requireLinux = false) {
  const version = pkgVersion(files.native, 'cargo', 'aquilum-native');
  const mirrors = {
    'core/Cargo.toml': pkgVersion(files.core, 'cargo', 'aquilum-core'),
    'Cargo.lock core': pkgVersion(files.lock, 'lock', 'aquilum-core'),
    'Cargo.lock native': pkgVersion(files.lock, 'lock', 'aquilum-native'),
  };
  const windows = JSON.parse(files.windows);
  if (windows.productName !== 'Akula') throw Error('Windows publisher identity differs from Akula');
  mirrors['Akula Windows packager'] = windows.version;
  if (requireLinux) {
    const linux = JSON.parse(files.linux);
    if (linux.productName !== 'Akula') throw Error('Linux publisher identity differs from Akula');
    mirrors['Akula Linux packager'] = linux.version;
  }
  for (const [where, actual] of Object.entries(mirrors)) {
    parts(actual);
    if (actual !== version) {
      throw Error(where + ' version ' + actual + ' differs from native ' + version);
    }
  }
  return version;
}

function replaceOnce(text, marker, oldVersion, next, label) {
  const before = marker + oldVersion + '"';
  const after = marker + next + '"';
  if (text.split(before).length !== 2) {
    throw Error('expected exactly one version field in ' + label);
  }
  return text.replace(before, after);
}

function planBump(files, target) {
  parts(target);
  const previous = check(files);
  if (compare(target, previous) <= 0) {
    throw Error('next Akula version must strictly exceed ' + previous);
  }
  const result = { ...files };
  result.native = replaceOnce(files.native, packageMarker('cargo', 'aquilum-native'), previous, target, PATHS.native);
  result.core = replaceOnce(files.core, packageMarker('cargo', 'aquilum-core'), previous, target, PATHS.core);
  for (const name of ['aquilum-core', 'aquilum-native']) {
    result.lock = replaceOnce(result.lock, packageMarker('lock', name), previous, target, PATHS.lock);
  }
  result.windows = replaceOnce(files.windows, '"version": "', previous, target, PATHS.windows);
  if (check(result) !== target) throw Error('planned version update failed verification');
  return result;
}

function nextPatch(version) {
  const [major, minor, patch] = parts(version);
  if (!Number.isSafeInteger(patch + 1)) throw Error('patch version overflow');
  return [major, minor, patch + 1].join('.');
}

function main(args = process.argv.slice(2)) {
  const action = args[0];
  if (!['check', 'check-linux', 'show', 'next-patch', 'bump', 'assert-newer'].includes(action)) {
    throw Error('usage: node scripts/akula-version.cjs check|check-linux|show|next-patch|bump X.Y.Z|assert-newer vX.Y.Z');
  }
  const files = load();
  const version = check(files, action === 'check-linux');
  if (action === 'show' || action === 'check' || action === 'check-linux') {
    process.stdout.write(version + '\n');
  } else if (action === 'next-patch') {
    process.stdout.write(nextPatch(version) + '\n');
  } else if (action === 'assert-newer') {
    const last = args[1];
    if (!last || !last.startsWith('v') || compare(version, last.slice(1)) <= 0) {
      throw Error('release v' + version + ' is not newer than published ' + last);
    }
    process.stdout.write('release monotonicity OK: ' + last + ' -> v' + version + '\n');
  } else {
    const updated = planBump(files, args[1]);
    // Validate every target in memory before touching any tracked file.
    for (const key of ['native', 'core', 'lock', 'windows']) {
      fs.writeFileSync(path.join(ROOT, PATHS[key]), updated[key]);
    }
    process.stdout.write('Akula Windows beta version: ' + version + ' -> ' + args[1] + '\n');
  }
}

if (require.main === module) {
  try { main(); }
  catch (error) {
    console.error('AKULA_VERSION_HOLD: ' + error.message);
    process.exitCode = 1;
  }
}

module.exports = { PATHS, load, parts, compare, check, planBump, nextPatch, main };
