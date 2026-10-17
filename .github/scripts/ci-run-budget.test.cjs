'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const {percentile,elapsed,analyze,report,fetchPage,run}=require('./ci-run-budget.cjs');
const t=n=>new Date(Date.UTC(2026,9,17,0,n)).toISOString();
const make=(name,id,minutes=7,conclusion='success',override={})=>({
  event:'pull_request',head_branch:'feature/'+id,head_sha:id,
  name,status:'completed',conclusion,run_attempt:1,
  created_at:t(0),updated_at:t(minutes),...override
});
test('nearest-rank percentiles and empty data',()=>{
  assert.equal(percentile([],0.9),null);
  assert.equal(percentile([9,1,3,5,7],0.5),5);
  assert.equal(percentile([9,1,3,5,7],0.9),9);
  assert.equal(elapsed(t(0),t(7)),7);
});
test('fanout counts distinct workflows, not repeats',()=>{
  const d=analyze([make('Application checks','a',8),make('Application checks','a',3,'cancelled'),
    make('Core performance baseline','a',3),make('Application checks','b',9),
    make('Push','c',60,'success',{event:'push'})],Date.parse(t(30)));
  assert.equal(d.recent,4);
  assert.equal(d.commits,2);
  assert.equal(d.fanout.median,1);
  assert.equal(d.runsPerCommit.p90,3);
  assert.equal(d.gate.n,2);
  assert.equal(d.cancelled,1);
});
test('thresholds warn after enough evidence',()=>{
  const samples=[];
  for(let i=0;i<11;i++){
    for(const name of ['Application checks','Core perf','Docs','Matrix'])
      samples.push(make(name,String(i),name==='Application checks'?11:3));
  }
  const result=analyze(samples,Date.parse(t(30)));
  assert.equal(result.budgets.length,3);
  assert.match(report(result),/Weekly CI/);
});
test('few observations are not treated as regressions',()=>{
  const d=analyze([make('Application checks','fresh',20),
    make('Application checks','old',20,'success',{created_at:'2025-01-01T00:00:00Z'})],Date.parse(t(30)));
  assert.equal(d.recent,1);
  assert.equal(d.budgets.length,0);
});
test('GitHub request is read-only and authenticated',async()=>{
  let requested;
  const fake=async(url,opts)=>{
    requested={url,opts};
    return {ok:true,json:async()=>({workflow_runs:[make('Application checks','x')]})};
  };
  const rows=await fetchPage('token','owner/repo',1,fake);
  assert.equal(rows.length,1);
  assert.match(requested.url,/event=pull_request/);
  assert.equal(requested.opts.headers.Authorization,'Bearer token');
  const summary=await run({GH_TOKEN:'token',GITHUB_REPOSITORY:'owner/repo'},fake);
  assert.match(summary,/Sample/);
});
