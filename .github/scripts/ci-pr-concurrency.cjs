'use strict';

const cp = require('node:child_process');
const fs = require('node:fs');

// Fail closed for NEW GitHub Actions PR workflows. Existing jobs retain their
// tested required checks/routing; the policy adds no new Actions runners.
const WORKFLOW = /^\.github\/workflows\/[^/\0]+\.ya?ml$/;

function topBlock(yaml, name) {
  const lines = String(yaml).replace(/\r\n/g, '\n').split('\n');
  const re = new RegExp('^(?:' + name + '|"' + name + '"|\\x27' + name + '\\x27):(?:\\s|$)');
  const i = lines.findIndex(line => re.test(line));
  if (i < 0) return [];
  const out = [lines[i]];
  for (let j = i + 1; j < lines.length; j++) {
    if (/^[^\s#][^:]*:\s*/.test(lines[j])) break;
    out.push(lines[j]);
  }
  return out;
}

function prEvents(yaml) {
  const on = topBlock(yaml, 'on');
  if (on.length === 0) return [];
  const inline = on[0].replace(/^(['"]?on['"]?):\s*/, '');
  return ['pull_request', 'pull_request_target'].filter(event => {
    const entry = new RegExp('^  [\\x27"]?' + event + '[\\x27"]?:');
    const inlineName = new RegExp('\\b' + event + '\\b');
    return inlineName.test(inline) || on.slice(1).some(line => entry.test(line));
  });
}
function prTrigger(yaml) { return prEvents(yaml).length > 0; }

function policyErrors(yaml, path) {
  if (!prTrigger(yaml)) return [];
  const lines = topBlock(yaml, 'concurrency');
  if (!lines.length) return [path + ': missing workflow-level concurrency'];
  const group = lines.find(line => /^  group:/.test(line)) || '';
  const cancel = lines.find(line => /^  cancel-in-progress:/.test(line)) || '';
  const clean = value => value.replace(/\s+#.*$/, '');
  const groupValue = clean(group.replace(/^  group:\s*/, ''));
  const cancelValue = clean(cancel.replace(/^  cancel-in-progress:\s*/, ''));
  const errors = [];
  // Match real Actions interpolation, not a descriptive YAML comment.
  if (!/\$\{\{[^}]*github\.workflow[^}]*\}\}/.test(groupValue) ||
      !/\$\{\{[^}]*github\.event\.pull_request\.number[^}]*github\.ref[^}]*\}\}/.test(groupValue)) {
    errors.push(path + ': group must isolate workflow and PR number, with ref fallback');
  }
  // Do not silently cancel workflow_dispatch, pushes or released artifacts.
  const condition = cancelValue.match(/^\$\{\{\s*(.*?)\s*\}\}$/)?.[1] || '';
  // Allow only explicit PR event equalities. Substring matching would accept
  // "github.event_name == 'pull_request' || true", cancelling manual/releases.
  const normalized = condition.replace(/\s+/g, '').replace(/"/g, "'");
  const expected = prEvents(yaml).map(event => "github.event_name=='" + event + "'");
  const options = expected.length === 1
    ? [expected[0]]
    : [expected.join('||'), expected.slice().reverse().join('||')];
  const actual = normalized.startsWith('(') && normalized.endsWith(')')
    ? normalized.slice(1,-1) : normalized;
  if (!options.includes(actual)) {
    errors.push(path + ': cancel-in-progress must cover PR triggers only');
  }
  if (/^  queue:\s*max\b/m.test(lines.join('\n'))) {
    errors.push(path + ': queue: max defeats obsolete-run coalescing');
  }
  return errors;
}

function git(cwd, ...args) {
  return cp.execFileSync('git', args, { cwd, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 });
}
function changedWorkflows(base, head = 'HEAD', cwd = process.cwd()) {
  for (const ref of [base, head]) {
    if (typeof ref !== 'string' || !/^[\w./-]+$/.test(ref) || ref.startsWith('-')) {
      throw Error('Untrusted Git revision');
    }
  }
  const raw = git(cwd, 'diff', '--name-status', '-z', '--no-renames', base, head, '--', '.github/workflows/');
  const fields = raw.split('\0').filter(Boolean);
  if (fields.length % 2 !== 0) throw Error('Unexpected git diff name-status output');
  const records = [];
  for (let i = 0; i < fields.length; i += 2) {
    if (!/^[AM]$/.test(fields[i])) continue;
    const path = fields[i + 1];
    if (WORKFLOW.test(path)) records.push({ status: fields[i], path });
  }
  return records;
}

function audit(base, head = 'HEAD', cwd = process.cwd()) {
  const records = changedWorkflows(base, head, cwd);
  const errors = [];
  let newPr = 0;
  let protectedModified = 0;
  for (const record of records) {
    const text = git(cwd, 'show', head + ':' + record.path);
    if (record.status === 'A') {
      if (prTrigger(text)) newPr++;
      errors.push(...policyErrors(text, record.path));
    } else {
      const before = git(cwd, 'show', base + ':' + record.path);
      // Modifying an existing workflow must not remove previously enforced
      // concurrency. We do not retroactively change legacy CI implementations.
      if (prTrigger(before) && topBlock(before, 'concurrency').length > 0) {
        protectedModified++;
        if (!prTrigger(text)) {
          // Required checks may depend on the PR trigger staying available.
          errors.push(record.path + ': removed existing pull_request trigger');
        } else {
          const newer = topBlock(text, 'concurrency');
          if (newer.length === 0) {
            errors.push(record.path + ': removed existing PR concurrency');
          } else {
            const prior = topBlock(before, 'concurrency');
            const priorCancel = prior.find(line => /^  cancel-in-progress:/.test(line));
            const currentCancel = newer.find(line => /^  cancel-in-progress:/.test(line));
            if (priorCancel && (!currentCancel || /^  cancel-in-progress:\s*false(?:\s|$)/.test(currentCancel))) {
              errors.push(record.path + ': removed existing PR cancellation');
            }
          }
        }
      }
    }
  }
  return { records: records.length, newPr, protectedModified, errors };
}

function main(argv) {
  const args = Object.fromEntries(argv.map(arg => {
    const match = arg.match(/^--(base|head)=([^\s]+)$/);
    if (!match) throw Error('Expected --base=SHA and --head=SHA');
    return [match[1], match[2]];
  }));
  if (!args.base) throw Error('--base required');
  const result = audit(args.base, args.head || 'HEAD');
  console.log('CI_PR_CONCURRENCY_AUDIT inspected=' + result.records +
    ' new_pr=' + result.newPr + ' protected_modified=' + result.protectedModified);
  if (result.errors.length > 0) {
    for (const error of result.errors) console.error('::error::' + error);
    process.exitCode = 1;
  }
}

if (require.main === module) {
  try { main(process.argv.slice(2)); }
  catch (error) {
    console.error('CI queue policy could not verify workflows: ' + error.message);
    process.exitCode = 1;
  }
}

module.exports = { topBlock, prEvents, prTrigger, policyErrors, changedWorkflows, audit, main };
