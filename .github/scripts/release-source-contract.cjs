'use strict';

// An intentionally strict release policy guard. Published packages remain
// LOCAL-only until a separate trusted OS custody installer/broker is proven.
const fs = require('node:fs');
const path = require('node:path');

function count(text, pattern) {
  return [...text.matchAll(pattern)].length;
}

function section(yaml, job) {
  const jobs = yaml.indexOf('\njobs:\n');
  if (jobs < 0) return '';
  const marker = '\n  ' + job + ':\n';
  const start = yaml.indexOf(marker, jobs);
  if (start < 0) return '';
  const remaining = yaml.slice(start + marker.length);
  const next = remaining.search(/\n  [a-z][a-z0-9-]*:\n/);
  return next < 0 ? remaining : remaining.slice(0, next);
}

function validateRelease(yaml) {
  const errors = [];
  const source = section(yaml, 'source');
  if (!source) errors.push('Missing immutable release source job');
  if (!/ref: \$\{\{\s*inputs\.ref\s*\}\}/.test(source)) {
    errors.push('Source job must resolve only the selected dispatch ref');
  }
  if (!/sha: \$\{\{\s*steps\.pin\.outputs\.sha\s*\}\}/.test(source)
      || !source.includes('git rev-parse --verify HEAD^{commit}')) {
    errors.push('Source job does not publish a verified Git commit SHA');
  }

  const pinnedRef = /ref: \$\{\{\s*needs\.source\.outputs\.sha\s*\}\}/g;
  const builds = /^\s*run:\s*cargo build --release -p aquilum-native[^\n]*$/gm;
  for (const job of ['windows', 'macos', 'linux', 'manifest']) {
    const body = section(yaml, job);
    if (!body) {
      errors.push('Missing release job ' + job);
      continue;
    }
    const needs = job === 'manifest'
      ? 'needs: [source, windows, macos, linux]'
      : 'needs: source';
    if (!body.includes(needs)) errors.push(job + ' does not depend on pinned source');
    if (count(body, pinnedRef) !== 1) {
      errors.push(job + ' does not checkout precisely one pinned source SHA');
    }
    if (!body.includes('test "$(git rev-parse HEAD)" = "$PINNED_SOURCE_SHA"')) {
      errors.push(job + ' does not verify actual checked-out Git SHA');
    }
    if (!body.includes('PINNED_SOURCE_SHA: ${{ needs.source.outputs.sha }}')) {
      errors.push(job + ' does not bind checkout verification to source job');
    }

    if (job !== 'manifest') {
      const found = [...body.matchAll(builds)].map(m => m[0].trim().replace(/^run:\s*/, ''));
      const expected = 'cargo build --release -p aquilum-native --features installed'
        + (job === 'macos' ? ' --target ${{ matrix.target }}' : '');
      if (found.length !== 1 || found[0] !== expected) {
        errors.push(job + ' production build is not the audited local-only feature set');
      }
    } else {
      for (const required of [
        'source_sha: $source',
        'custody_mode: "local-only"',
        'cargo_features: ["installed"]',
        'installer_host_registry_provisioned: false',
        'remote_s3_mutations_enabled: false',
        '> custody-release.json',
      ]) {
        if (!body.includes(required)) errors.push('manifest missing custody/provenance field: ' + required);
      }
    }
  }

  if (count(yaml, builds) !== 3) {
    errors.push('Release should contain exactly three production build commands');
  }
  if (count(yaml, pinnedRef) !== 4) {
    errors.push('Every platform and manifest must checkout the pinned commit');
  }
  if (yaml.includes('AKULA_INSTALLED_HOST_CUSTODY_DB')
      || /--all-features|sync-installed-host|sync-s3/.test(yaml)) {
    errors.push('Unsafe custody/S3 feature or runtime selection in local-only release');
  }
  return errors;
}

if (require.main === module) {
  const input = path.resolve(__dirname, '..', 'workflows', 'release.yml');
  const errors = validateRelease(fs.readFileSync(input, 'utf8'));
  if (errors.length) {
    for (const error of errors) process.stderr.write('RELEASE CONTRACT: ' + error + '\n');
    process.exitCode = 1;
  } else {
    process.stdout.write('Release source and local-only custody policy verified\n');
  }
}

module.exports = { validateRelease };
