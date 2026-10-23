'use strict';
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { calculateBudget, oldestCandidates, enforceAdmission } = require('./pr-queue-budget.cjs');

function pr(n, opts = {}) {
  return {
    number: n, state: opts.state || 'open', draft: opts.draft ?? true,
    base: { ref: opts.base || 'main' },
    created_at: opts.created || '2026-10-22T09:00:00Z',
    closed_at: opts.closed || null,
    updated_at: opts.updated || '2026-10-23T10:00:00Z',
  };
}
function queue(n) { return Array.from({ length: n }, (_, i) => pr(i + 1)); }

test('initial cap 140 and goal 50 count Draft and stacked PRs', () => {
  const all = queue(136);
  all[1].base.ref = 'feature/stack';
  all[2].draft = false;
  const result = calculateBudget(all);
  assert.deepEqual([result.openCount, result.draftCount, result.stackedCount, result.ceiling, result.goal],
    [136, 135, 1, 140, 50]);
});

test('closing each legacy PR ratchets cap down exactly one', () => {
  const all = [...queue(135), pr(136, {state:'closed', closed:'2026-10-23T17:00:00Z'})];
  const result = calculateBudget(all);
  assert.deepEqual([result.openCount, result.retiredLegacy, result.ceiling], [135, 1, 139]);
});

test('floor at 50 and never counts pre-cutover closures', () => {
  const old = Array.from({length: 110}, (_,i) => pr(i+1, {state:'closed', closed:'2026-10-23T17:00:00Z'}));
  old.push(pr(111, {state:'closed', closed:'2026-10-22T18:00:00Z'}));
  old.push(pr(112, {state:'closed', closed:'2026-10-23T17:00:00Z', created:'2026-10-23T16:13:00Z'}));
  assert.equal(calculateBudget(old).ceiling, 50);
  assert.equal(calculateBudget([old[110],old[111]]).retiredLegacy, 0);
});

test('a reopened legacy PR is not falsely counted as retired', () => {
  const all = [pr(1), pr(2, {state:'closed', closed:'2026-10-23T17:00:00Z'})];
  assert.equal(calculateBudget(all).retiredLegacy, 1);
});

test('triage prioritizes stacked open PRs but never closes them', () => {
  const all = [pr(1, {updated:'2026-10-20T00:00:00Z'}),
    pr(2, {base:'feature/dependency', updated:'2026-10-22T00:00:00Z'}),
    pr(3, {state:'closed', closed:'2026-10-23T18:00:00Z'}),
    pr(4, {base:'feature/dependency', updated:'2026-10-21T00:00:00Z'})];
  assert.deepEqual(oldestCandidates(all).map(x=>x.number),[4,2,1]);
});

function mock(initial, fresh = initial, prState = 'open') {
  const calls = { comments:[], closed:[], listings:0, prGets:0 };
  const api = {
    rest: {
      pulls: {
        list: Symbol('listPulls'),
        get: async () => { calls.prGets++; return {data: {state:prState}}; },
        update: async args => { calls.closed.push(args); },
      },
      issues: { createComment: async args => { calls.comments.push(args); } },
    },
    paginate: async () => { calls.listings++; return calls.listings===1 ? initial : fresh; },
  };
  const messages = [];
  const core = {info:s=>messages.push(s), warning:s=>messages.push(s)};
  const context = {repo: {owner:'example',repo:'Akula'}, payload: {action:'opened',pull_request:{number:1000}}};
  return { github:api, context, core, calls, messages };
}

test('new PR below cap is admitted with no comment or mutation', async () => {
  const s = mock(queue(136));
  const result = await enforceAdmission(s);
  assert.equal(result.admitted,true);
  assert.equal(s.calls.listings,1);
  assert.equal(s.calls.closed.length,0);
});

test('new PR above 140 is commented and closed without deleting its branch', async () => {
  const s = mock(queue(141));
  const result = await enforceAdmission(s);
  assert.equal(result.reason,'over_ceiling');
  assert.equal(s.calls.listings,2);
  assert.equal(s.calls.comments[0].issue_number,1000);
  assert.deepEqual(s.calls.closed,[{owner:'example',repo:'Akula',pull_number:1000,state:'closed'}]);
});

test('second snapshot below cap preserves PR after a concurrent merge', async () => {
  const s=mock(queue(141),queue(140));
  assert.equal((await enforceAdmission(s)).admitted,true);
  assert.equal(s.calls.closed.length,0);
});

test('post-cutover legacy reductions enforce a lower-than-140 cap', async () => {
  const legacy = Array.from({length:90},(_,i)=>pr(i+1,{state:'closed',closed:'2026-10-23T17:00:00Z'}));
  const s=mock([...queue(51),...legacy]);
  assert.equal((await enforceAdmission(s)).reason,'over_ceiling');
  assert.equal(s.calls.closed.length,1);
});

test('already closed and invalid events never close any PR', async () => {
  const s=mock(queue(141),queue(141),'closed');
  assert.equal((await enforceAdmission(s)).reason,'already_closed');
  assert.equal(s.calls.listings,0);
  assert.equal(s.calls.closed.length,0);
  s.context.payload.action='synchronize';
  await assert.rejects(enforceAdmission(s),/opened\/reopened/);
});

test('API failure does not mutate other PRs', async () => {
  const s=mock(queue(141));
  s.github.paginate=async()=>{throw new Error('GitHub unavailable')};
  await assert.rejects(enforceAdmission(s),/GitHub unavailable/);
  assert.equal(s.calls.closed.length,0);
});
