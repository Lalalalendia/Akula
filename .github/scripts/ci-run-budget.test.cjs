'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const {percentile,elapsed,analyze,report,fetchPage,sampleRecent,run,MAX_PAGES,
  analyzeJobTimings,sampleJobTimings,jobTimingReport,MAX_TIMING_RUNS_PER_WORKFLOW}=require('./ci-run-budget.cjs');
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

test('completed skipped workflows count toward triggers, not useful workload',()=>{
  const sample=[
    make('Application checks','a',7),
    make('Core performance baseline','a',1,'skipped'),
    make('Paired core performance comparison','a',1,'skipped'),
    make('Branch hygiene','a',1)
  ];
  const d=analyze(sample,Date.parse(t(30)));
  assert.equal(d.triggeredFanout.max,4);
  assert.equal(d.fanout.max,2);
  assert.equal(d.skipped,2);
  assert.equal(d.gate.n,1);
  assert.match(report({...d,pages:1,truncated:false}),/NON-SKIPPED/);
});
test('weekly collector traverses beyond the old 200-run cap',async()=>{
  let visited=0;
  const fake=async(url)=>{
    const page=Number(new URL(url).searchParams.get('page'));
    visited++;
    const count=page===3?37:100;
    return {ok:true,json:async()=>({workflow_runs:Array.from({length:count},(_,i)=>
      make('Application checks',page+'-'+i,2))})};
  };
  const now=Date.parse(t(30));
  const sample=await sampleRecent('token','owner/repo',fake,now);
  assert.equal(visited,3);
  assert.equal(sample.runs.length,237);
  assert.equal(sample.truncated,false);
  const summary=await run({GH_TOKEN:'token',GITHUB_REPOSITORY:'owner/repo'},fake,now);
  assert.match(summary,/237 PR workflow runs/);
  assert.match(summary,/3 API page/);
});
test('oldest timestamp ends pagination despite a full page',async()=>{
  let visited=0;
  const fake=async()=>{
    visited++;
    const rows=Array.from({length:100},(_,i)=>make('Application checks',String(i)));
    rows[99].created_at='2025-01-01T00:00:00Z';
    return {ok:true,json:async()=>({workflow_runs:rows})};
  };
  const sample=await sampleRecent('token','owner/repo',fake,Date.parse(t(30)));
  assert.equal(visited,1);
  assert.equal(sample.pages,1);
  assert.equal(sample.truncated,false);
  assert.equal(analyze(sample.runs,Date.parse(t(30))).recent,99);
});
test('bounded maximum reports truncation rather than pretending full coverage',async()=>{
  let visited=0;
  const fake=async()=>{
    visited++;
    return {ok:true,json:async()=>({workflow_runs:Array.from({length:100},(_,i)=>
      make('Application checks','same-'+i))})};
  };
  const now=Date.parse(t(30));
  const sample=await sampleRecent('token','owner/repo',fake,now);
  assert.equal(visited,MAX_PAGES);
  assert.equal(sample.runs.length,MAX_PAGES*100);
  assert.equal(sample.truncated,true);
  const result={...analyze(sample.runs,now),pages:sample.pages,truncated:sample.truncated};
  assert.match(report(result),/WARNING: sample stopped/);
});

test('job timing uses real job timestamps and excludes skipped or invalid jobs',()=>{
  const jobs=[
    {name:'Required Rust tests',status:'completed',conclusion:'success',
      created_at:t(0),started_at:t(4),completed_at:t(5)},
    {name:'Required Rust tests',status:'completed',conclusion:'success',
      created_at:t(0),started_at:t(2),completed_at:t(5)},
    {name:'Skipped lane',status:'completed',conclusion:'skipped',
      created_at:t(0),started_at:t(0),completed_at:t(0)},
    {name:'Invalid lane',status:'completed',conclusion:'failure',
      created_at:t(6),started_at:t(4),completed_at:t(9)}
  ];
  const d=analyzeJobTimings([{workflow:'Application checks',jobs}]);
  assert.equal(d.sampledRuns,1);
  assert.equal(d.completedJobs,2);
  assert.deepEqual(d.rows.map(r=>r.name),['Required Rust tests']);
  assert.equal(d.rows[0].n,2);
  assert.equal(d.rows[0].queueMedian,2);
  assert.equal(d.rows[0].queueP90,4);
  assert.equal(d.rows[0].durationMedian,1);
  assert.match(jobTimingReport(d),/Queue p50/);
  assert.match(jobTimingReport(d),/2\.0 \/ 4\.0/);
});
test('sampled job requests are capped, authenticated, and limited to completed PR runs',async()=>{
  const requests=[];
  const fake=async(url,opts)=>{
    requests.push({url,opts});
    return {ok:true,json:async()=>({total_count:1,jobs:[{
      name:'Required Rust tests',status:'completed',conclusion:'success',
      created_at:t(0),started_at:t(4),completed_at:t(6)}]})};
  };
  const runs=[
    ...Array.from({length:MAX_TIMING_RUNS_PER_WORKFLOW+3},(_,i)=>make('Application checks',String(i),3,'success',{id:100+i})),
    make('Docs and installer checks','docs',1,'success',{id:200}),
    make('Application checks','incomplete',1,null,{id:201,status:'queued'}),
    make('Non-selected workflow','other',1,'success',{id:202})
  ];
  const d=await sampleJobTimings('token','owner/repo',runs,fake,Date.parse(t(30)));
  assert.equal(requests.length,MAX_TIMING_RUNS_PER_WORKFLOW+1);
  assert.equal(d.sampledRuns,MAX_TIMING_RUNS_PER_WORKFLOW+1);
  assert.equal(d.completedJobs,MAX_TIMING_RUNS_PER_WORKFLOW+1);
  assert.ok(requests.every(x=>x.url.includes('/actions/runs/')&&x.url.endsWith('/jobs?per_page=100')));
  assert.ok(requests.every(x=>x.opts.headers.Authorization==='Bearer token'));
  assert.ok(requests.every(x=>!x.url.includes('/201/')&&!x.url.includes('/202/')));
});
test('job timing API failure does not hide the existing weekly audit',async()=>{
  const fake=async(url)=>{
    if(url.includes('/jobs?'))return {ok:false,status:503};
    return {ok:true,json:async()=>({workflow_runs:[
      make('Application checks','x',2,'success',{id:45})]})};
  };
  const summary=await run({
    GH_TOKEN:'token',GITHUB_REPOSITORY:'owner/repo',CI_JOB_TIMING_SAMPLE:'1'
  },fake,Date.parse(t(30)));
  assert.match(summary,/Weekly CI/);
  assert.match(summary,/Sampling unavailable: Actions jobs API HTTP 503/);
});

test('specialized PR CI isolates obsolete PR runs and preserves manual jobs',()=>{
  const fs=require('node:fs');
  const path=require('node:path');
  const files=[
    'core-server-bootstrap-b1b0-staging.yml',
    'core-server-bootstrap-b1b4a-cas-fence.yml',
    'core-server-bootstrap-b1b4b-os-witness.yml',
    'core-server-bootstrap-b1c0-fixed-node.yml',
    'core-server-bootstrap-b1d0-windows-setup.yml',
    'core-node-d1-loopback.yml',
    'core-node-d15-catalog.yml',
    'core-headless-smoke.yml',
    'sync-installed-os-owner-witness.yml',
    'core-server-bootstrap-b1b4b1-witness-ipc.yml',
    'sync-text-delta-rust.yml',
  ];
  // These two workflows were deliberately centralized as workflow_call;
  // the required PR gate owns their PR routing now.
  const centrallyRouted=new Set([
    'sync-installed-os-owner-witness.yml',
    'sync-text-delta-rust.yml',
  ]);
  for(const file of files){
    const yaml=fs.readFileSync(path.join(__dirname,'../workflows',file),'utf8');
    if(centrallyRouted.has(file)){
      assert.match(yaml,/^  workflow_call:/m,file);
      assert.doesNotMatch(yaml,/^  pull_request:/m,file);
    }else{
      assert.match(yaml,/^  pull_request:/m,file);
    }
    assert.match(yaml,/^  workflow_dispatch:/m,file);
    assert.match(yaml,/^permissions:$/m,file);
    assert.match(yaml,/^  contents: read$/m,file);
    assert.match(yaml,/^concurrency:$/m,file);
    // Exact prefix is not a safety invariant: prefixes intentionally vary.
    // Workflow + PR number isolate parallel PRs; run_id isolates manual jobs.
    assert.match(yaml,
      /^  group: [a-zA-Z0-9_-]+-\$\{\{ github\.workflow \}\}-\$\{\{ github\.event\.pull_request\.number \|\| github\.run_id \}\}$/m,
      file);
    assert.match(yaml,/^  cancel-in-progress: true$/m,file);
  }
});
