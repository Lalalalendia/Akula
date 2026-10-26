'use strict';
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { MAX_OPEN_PRS, calculateBudget, oldestCandidates, listPulls, enforceAdmission } = require('./pr-queue-budget.cjs');

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

test('fixed cap 140 counts Draft and stacked PRs',()=>{
  const full=queue(140);
  full[0].base.ref='feature/dependency';full[1].draft=false;
  assert.equal(MAX_OPEN_PRS,140);
  assert.deepEqual([calculateBudget(full).ceiling,calculateBudget(full).draftCount,
    calculateBudget(full).stackedCount,calculateBudget(full).overflow],[140,139,1,0]);
  assert.equal(calculateBudget(queue(141)).overflow,1);
});

test('legacy and post-cutover closures release capacity without shrinking 140',()=>{
  const closedOld=pr(1000,{state:'closed',closed:'2026-10-23T17:00:00Z'});
  const closedNew=pr(1001,{state:'closed',closed:'2026-10-24T12:00:00Z',
    created:'2026-10-24T08:00:00Z'});
  const full=queue(140);
  assert.deepEqual([calculateBudget([...full,closedOld,closedNew]).openCount,
    calculateBudget([...full,closedOld,closedNew]).ceiling],[140,140]);
  const reduced=calculateBudget([...full.slice(1),closedOld,closedNew]);
  assert.deepEqual([reduced.openCount,reduced.ceiling,reduced.overflow],[139,140,0]);
});

test('no historical backlog or reopen can ratchet the limit',()=>{
  const closed=Array.from({length:110},(_,i)=>pr(i+500,{
    state:'closed',closed:'2026-10-23T17:00:00Z'}));
  assert.deepEqual([calculateBudget([...queue(140),...closed]).ceiling,
    calculateBudget([...queue(140),...closed]).overflow],[140,0]);
  const reopened=pr(900,{created:'2026-10-17T00:00:00Z'});
  assert.equal(calculateBudget([...queue(139),reopened]).overflow,0);
  assert.equal(calculateBudget([...queue(140),reopened]).overflow,1);
});

test('paginated GitHub inventory reads only open PRs',async()=>{
  const calls=[];
  const github={rest:{pulls:{list:'list'}},paginate:async(fn,args)=>{
    calls.push({fn,args});return queue(140);
  }};
  assert.equal((await listPulls(github,'org','Akula')).length,140);
  assert.deepEqual(calls,[{fn:'list',args:{
    owner:'org',repo:'Akula',state:'open',per_page:100
  }}]);
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

test('new PR at cap 140 is admitted with no mutation', async () => {
  const s = mock(queue(140));
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

test('historical closings never lower admission ceiling',async()=>{
  const closed=Array.from({length:90},(_,i)=>pr(i+1000,{
    state:'closed',closed:'2026-10-23T17:00:00Z'
  }));
  const st=mock([...queue(140),...closed]);
  assert.equal((await enforceAdmission(st)).admitted,true);
  assert.equal(st.calls.closed.length,0);
});

test('over-limit explanation says fixed 140, not shrinking target',async()=>{
  const st=mock(queue(141));
  assert.equal((await enforceAdmission(st)).reason,'over_ceiling');
  assert.match(st.calls.comments[0].body,/fixed maximum: 140/);
  assert.match(st.calls.comments[0].body,/does not shrink/);
  assert.doesNotMatch(st.calls.comments[0].body,/target: 50|absolute maximum: 130/);
});

test('global admission checks are serialized and never cancel each other',()=>{
  const fs=require('node:fs'),path=require('node:path');
  const wf=fs.readFileSync(path.join(__dirname,'../workflows/branch-hygiene.yml'),'utf8');
  const section=wf.split('  pr-admission:')[1]?.split('  audit-or-prune:')[0];
  assert.ok(section);
  assert.match(section,/group: akula-pr-admission-global/);
  assert.match(section,/cancel-in-progress: false/);
  assert.match(section,/github.event.repository.default_branch/);
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
