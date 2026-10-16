'use strict';
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { classifyBranches, run } = require('./branch-hygiene.cjs');

const repository = { full_name: 'example/Akula', default_branch: 'main' };
const makeBranch = (name, sha = 'sha', protectedBranch = false) => ({
  name, protected: protectedBranch, commit: { sha },
});
const makePr = (ref, { sha = 'sha', state = 'closed', merged = true, headRepo = repository.full_name } = {}) => ({
  state, merged_at: merged ? '2026-10-17T00:00:00Z' : null,
  head: { ref, sha, repo: { full_name: headRepo } },
});

test('only unchanged merged branches are candidates', () => {
  const branches = [
    makeBranch('main'), makeBranch('merged'), makeBranch('reused', 'new'),
    makeBranch('closed-only'), makeBranch('unlinked'), makeBranch('active'),
    makeBranch('protected', 'sha', true), makeBranch('fork'),
  ];
  const pulls = [
    makePr('merged'), makePr('reused', { sha: 'old' }),
    makePr('closed-only', { merged: false }), makePr('active', { state: 'open', merged: false }),
    makePr('protected'), makePr('fork', { headRepo: 'other/fork' }),
  ];
  const plan = classifyBranches({ branches, pulls, repository });
  assert.deepEqual(plan.candidates.map((x) => x.name), ['merged']);
  assert.deepEqual(Object.fromEntries(plan.held.map((x) => [x.name, x.reason])), {
    main: 'default_branch', reused: 'merged_pr_sha_changed',
    'closed-only': 'closed_unmerged_pr', unlinked: 'no_matching_pr',
    active: 'open_pr', protected: 'protected', fork: 'no_matching_pr',
  });
});

test('an open PR sharing a branch blocks cleanup even if an earlier PR merged', () => {
  const plan = classifyBranches({
    branches: [makeBranch('reused')],
    pulls: [makePr('reused'), makePr('reused', { state: 'open', merged: false })],
    repository,
  });
  assert.equal(plan.candidates.length, 0);
  assert.equal(plan.held[0].reason, 'open_pr');
});

function mockServices({ open = [], currentSha = 'sha' } = {}) {
  const deletions = [];
  const github = {
    rest: {
      repos: {
        get: async () => ({ data: repository }),
        listBranches: 'listBranches',
      },
      pulls: {
        list: (...args) => {
          if (args[0].state === 'open') return Promise.resolve({ data: open });
          throw new Error('Unexpected pulls.list');
        },
      },
      git: {
        getRef: async () => ({ data: { object: { type: 'commit', sha: currentSha } } }),
        deleteRef: async ({ ref }) => { deletions.push(ref); },
      },
    },
    paginate: async (method) => {
      if (method === 'listBranches') return [makeBranch('merged'), makeBranch('main')];
      if (typeof method === 'function') return [makePr('merged')];
      throw new Error('Unexpected paginate');
    },
  };
  const messages = [];
  const core = {
    info: (msg) => messages.push(msg),
    summary: { addRaw: (msg) => { messages.push(msg); return { write: async () => {} }; } },
  };
  return { github, core, context: { repo: { owner: 'example', repo: 'Akula' } }, deletions, messages };
}

test('audit mode never deletes anything', async () => {
  const s = mockServices();
  await run({ ...s, mode: 'audit' });
  assert.deepEqual(s.deletions, []);
});

test('prune removes an unchanged merged branch', async () => {
  const s = mockServices();
  const result = await run({ ...s, mode: 'prune' });
  assert.deepEqual(s.deletions, ['heads/merged']);
  assert.deepEqual(result.deleted, ['merged']);
});

test('prune skips when a branch was updated since enumeration', async () => {
  const s = mockServices({ currentSha: 'new-head' });
  const result = await run({ ...s, mode: 'prune' });
  assert.equal(result.skipped[0].reason, 'head_sha_changed');
  assert.deepEqual(s.deletions, []);
});

test('prune skips a branch if another PR opened after enumeration', async () => {
  const s = mockServices({ open: [{ number: 100 }] });
  const result = await run({ ...s, mode: 'prune' });
  assert.equal(result.skipped[0].reason, 'new_open_pr');
  assert.deepEqual(s.deletions, []);
});

test('prune respects a hard per-run limit', async () => {
  const s = mockServices();
  await run({ ...s, mode: 'prune', maxDeletes: 0 });
  assert.deepEqual(s.deletions, []);
});
