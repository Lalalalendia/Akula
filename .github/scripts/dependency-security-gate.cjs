'use strict';

// Do not equate a successful daily decision with a completed RustSec audit.
// This is the fail-closed reader for the watchdog's actual audit job.
const cp = require('node:child_process');
const fs = require('node:fs');

const WORKFLOW = '.github/workflows/dependency-security.yml';
const JOB = 'RustSec dependency audit';
const MAX_AGE_MS = 96 * 60 * 60 * 1000;
const ALLOWED_EVENTS = new Set(['schedule', 'workflow_dispatch', 'push']);
const LOCAL_SCAN_PATHS = new Set([
  WORKFLOW,
  '.github/workflows/app-checks.yml',
  '.github/scripts/dependency-security-gate.cjs',
  '.github/scripts/dependency-security-gate.test.cjs',
]);
// A bootstrap may introduce ONLY the security gate itself. It cannot add
// unrelated product changes while bypassing the already-failing baseline.
const BOOTSTRAP_FILES = new Set(LOCAL_SCAN_PATHS);

function mustScanLocally(paths) {
  return paths.some(p =>
    LOCAL_SCAN_PATHS.has(p) ||
    /(^|\/)Cargo\.(?:toml|lock)$/.test(p) ||
    /(^|\/)audit\.toml$/.test(p)
  );
}

function verdict(run, jobs, nowMs) {
  if (!run || run.head_branch !== 'main' || !ALLOWED_EVENTS.has(run.event)) {
    return { kind: 'skip', reason: 'not the default-branch watchdog' };
  }
  if (run.status !== 'completed') {
    return { kind: 'deny', reason: 'watchdog run ' + run.id + ' is not completed' };
  }
  const audit = jobs.find(j => j.name === JOB);
  if (!audit) {
    return { kind: 'deny', reason: 'watchdog audit job missing from run ' + run.id };
  }
  if (audit.conclusion === 'skipped' && run.conclusion === 'success') {
    return { kind: 'skip', reason: 'non-due day, not an audit result' };
  }
  if (run.conclusion !== 'success' || audit.conclusion !== 'success') {
    return { kind: 'deny', reason: 'watchdog audit failed or unavailable in run ' + run.id };
  }
  const completed = Date.parse(audit.completed_at || run.updated_at || '');
  if (!Number.isFinite(completed) || completed > nowMs + 300000) {
    return { kind: 'deny', reason: 'invalid watchdog audit completion timestamp' };
  }
  if (nowMs - completed > MAX_AGE_MS) {
    return { kind: 'deny', reason: 'last successful RustSec audit is older than 96 hours' };
  }
  return { kind: 'allow', reason: 'fresh successful RustSec scan in run ' + run.id };
}

function git(...args) {
  return cp.execFileSync('git', args, { encoding: 'utf8' }).trim();
}

function missingFromBase(base) {
  try {
    git('cat-file', '-e', base + ':' + WORKFLOW);
    return false;
  } catch (error) {
    if (error.status === 128) return true;
    throw error;
  }
}

async function readGitHub(url, token) {
  const response = await fetch(url, {
    headers: {
      authorization: 'Bearer ' + token,
      accept: 'application/vnd.github+json',
      'x-github-api-version': '2022-11-28',
    },
    signal: AbortSignal.timeout(15000),
  });
  if (!response.ok) {
    throw new Error('GitHub Actions API HTTP ' + response.status + ' on ' + new URL(url).pathname);
  }
  return response.json();
}

// The workflow-specific runs endpoint has returned an empty list even when
// a completed main-branch watchdog exists. Read the branch run list instead,
// but admit ONLY the exact watchdog workflow path, not a similarly named job.
function watchdogRuns(runs) {
  if (!Array.isArray(runs)) throw new Error('missing workflow_runs');
  return runs.filter(run =>
    run && run.path === WORKFLOW &&
    run.head_branch === 'main' && ALLOWED_EVENTS.has(run.event)
  );
}

async function checkLatestAudit(repo, token) {
  const root = 'https://api.github.com/repos/' + repo;
  // GitHub lists newest runs first. If a newer actual scan failed or is still
  // running, verdict() denies immediately; never fall back to an older PASS.
  // A truncated 100-run main window with no audit must remain HOLD.
  const data = await readGitHub(root + '/actions/runs?branch=main&per_page=100', token);
  for (const run of watchdogRuns(data.workflow_runs)) {
    const page = await readGitHub(root + '/actions/runs/' + run.id + '/jobs?per_page=100', token);
    if (!Array.isArray(page.jobs)) throw new Error('missing watchdog jobs');
    const decision = verdict(run, page.jobs, Date.now());
    if (decision.kind === 'skip') continue;
    if (decision.kind === 'deny') throw new Error(decision.reason);
    return decision.reason;
  }
  throw new Error('no fresh actual RustSec audit on main (skipped days do not count)');
}

async function main() {
  const baseArg = process.argv.find(x => x.startsWith('--base='));
  const headArg = process.argv.find(x => x.startsWith('--head='));
  if (!baseArg || !headArg) throw new Error('expected --base= and --head=');
  const base = baseArg.slice(7), head = headArg.slice(7);
  const changed = git('diff', '--name-only', base, head).split('\n').filter(Boolean);
  let local = mustScanLocally(changed);
  let bootstrap = false;
  if (missingFromBase(base)) {
    // The one-time bootstrap PR cannot inspect a workflow not yet in main.
    // Require an actual full local audit; never mark an absent remote PASS.
    if (!fs.existsSync(WORKFLOW) || !changed.includes(WORKFLOW)) {
      throw new Error('watchdog missing from base and not introduced by this PR');
    }
    if (changed.some(path => !BOOTSTRAP_FILES.has(path))) {
      throw new Error('bootstrap PR contains non-security files: refuse initial gate exemption');
    }
    bootstrap = true;
    local = true;
    // The one-time installation is allowed ONLY after both real scans record
    // existing findings or prove clean. Transient/network errors remain fatal.
    console.error('BOOTSTRAP: install fail-closed watchdog while recording existing RustSec findings');
  } else {
    const repo = process.env.GITHUB_REPOSITORY;
    const token = process.env.GH_TOKEN;
    if (!repo || !token || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo)) {
      throw new Error('missing authenticated repository Actions reader');
    }
    try {
      console.error('SECURITY: ' + await checkLatestAudit(repo, token));
    } catch (error) {
      // Allow a remediation PR to replace a bad main lockfile/policy only if
      // its OWN fresh full cargo audit succeeds in subsequent required steps.
      // No local dependency/policy change means mandatory global HOLD.
      if (!local) throw error;
      console.error('MAIN SECURITY HOLD: ' + error.message +
        '; dependency/policy PR must now pass an independent full local RustSec audit');
    }
  }
  process.stdout.write('scan_required=' + (local ? 'true' : 'false') + '\n');
  process.stdout.write('bootstrap_mode=' + (bootstrap ? 'true' : 'false') + '\n');
}

if (require.main === module) {
  main().catch(error => {
    console.error('::error title=Dependency security HOLD::' + error.message);
    process.exitCode = 1;
  });
}

module.exports = { mustScanLocally, verdict, watchdogRuns, MAX_AGE_MS, BOOTSTRAP_FILES };
