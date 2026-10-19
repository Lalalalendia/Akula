'use strict';
const fs=require('node:fs');
const MAX_PAGES=12;
const DAYS=7;

function percentile(nums,fraction) {
  const sorted=nums.filter(Number.isFinite).sort((a,b)=>a-b);
  return sorted.length?sorted[Math.max(0,Math.ceil(sorted.length*fraction)-1)]:null;
}
function elapsed(a,b) {
  const start=Date.parse(a||''),end=Date.parse(b||'');
  return Number.isFinite(start)&&Number.isFinite(end)&&end>=start?(end-start)/60000:null;
}
function distribution(values) {
  return {median:percentile(values,.5),p90:percentile(values,.9),
    max:values.length?Math.max(...values):null};
}
function analyze(runs,now=Date.now()) {
  const after=+now-DAYS*86400000;
  const recent=runs.filter(r=>r.event==='pull_request'&&Date.parse(r.created_at)>=after);
  const bySha=new Map(),durations=new Map();
  let success=0,fail=0,cancelled=0,skipped=0;
  for(const r of recent) {
    if(r.head_branch&&r.head_sha) {
      const key=r.head_branch+'|'+r.head_sha;
      if(!bySha.has(key)) bySha.set(key,{
        triggered:new Set(),notSkipped:new Set(),runs:0
      });
      const group=bySha.get(key);
      group.triggered.add(r.name);
      // A workflow marked skipped consumes no useful runner time.
      // Cancelled workflows are retained: they may have executed before cancellation.
      if(r.conclusion!=='skipped') group.notSkipped.add(r.name);
      group.runs++;
    }
    if(r.status!=='completed') continue;
    if(r.conclusion==='success') success++;
    if(r.conclusion==='failure') fail++;
    if(r.conclusion==='cancelled') cancelled++;
    if(r.conclusion==='skipped') skipped++;
    if(!['success','failure'].includes(r.conclusion)) continue;
    const mins=elapsed(r.created_at,r.updated_at);
    if(mins===null) continue;
    if(!durations.has(r.name)) durations.set(r.name,[]);
    durations.get(r.name).push(mins);
  }
  const gate=durations.get('Application checks')||[];
  const triggered=[...bySha.values()].map(g=>g.triggered.size);
  const notSkipped=[...bySha.values()].map(g=>g.notSkipped.size);
  const duplicates=[...bySha.values()].map(g=>g.runs);
  const budgets=[];
  if(gate.length>=10&&percentile(gate,.9)>10) budgets.push('Main PR gate p90 exceeds 10 minutes');
  if(gate.length>=10&&percentile(gate,.5)>8) budgets.push('Main PR gate median exceeds 8 minutes');
  if(notSkipped.length>=10&&percentile(notSkipped,.9)>3) budgets.push('Non-skipped PR workflow fan-out p90 exceeds 3');
  return {recent:recent.length,commits:bySha.size,success,fail,cancelled,skipped,
    gate:{n:gate.length,median:percentile(gate,.5),p90:percentile(gate,.9)},
    fanout:distribution(notSkipped),
    triggeredFanout:distribution(triggered),
    runsPerCommit:distribution(duplicates),
    workflows:[...durations.entries()].map(([name,arr])=>({name,n:arr.length,median:percentile(arr,.5),p90:percentile(arr,.9)}))
      .sort((a,b)=>b.n-a.n).slice(0,12),budgets};
}
function format(n){return n==null?'n/a':n.toFixed(1);}
function report(d){
  const out=['## Weekly CI / fan-out budget (diagnostic)','',
    'Sample: last '+DAYS+' days, '+d.pages+' API page(s), '+d.recent+' PR workflow runs, '+d.commits+' commit+branch pairs.','',
    '- Main PR gate: n='+d.gate.n+', median '+format(d.gate.median)+' min, p90 '+format(d.gate.p90)+' min.',
    '- Distinct NON-SKIPPED workflows per PR SHA: median '+format(d.fanout.median)+', p90 '+format(d.fanout.p90)+', max '+format(d.fanout.max)+'.',
    '- Distinct TRIGGERED workflows per PR SHA (including skipped): median '+format(d.triggeredFanout.median)+', p90 '+format(d.triggeredFanout.p90)+'.',
    '- Raw runs per PR SHA (including repeated checks): median '+format(d.runsPerCommit.median)+', p90 '+format(d.runsPerCommit.p90)+'.',
    '- Completed: '+d.success+' success, '+d.fail+' failure, '+d.cancelled+' cancelled, '+d.skipped+' skipped.','',
    '| Workflow | Samples | Median minutes | p90 minutes |',
    '| --- | ---: | ---: | ---: |'];
  for(const w of d.workflows) out.push('| '+w.name.replace(/[|\r\n]/g,' ')+' | '+w.n+' | '+format(w.median)+' | '+format(w.p90)+' |');
  out.push('','### Advisory warnings');
  if(d.budgets.length) for(const warning of d.budgets) out.push('- '+warning);
  else out.push('None in this sample.');
  if(d.truncated) out.push('- WARNING: sample stopped at '+MAX_PAGES*100+' runs before covering the full 7-day window; percentiles may be biased.');
  out.push('','Wall-clock durations include queue delay and are NOT billed runner time. A cancelled workflow may never start a runner; the non-skipped metric is an upper bound. Jobs can run in parallel. No budget warning blocks merging.','');
  return out.join('\n');
}
async function fetchPage(token,repo,page,fetchImpl=fetch){
  const url='https://api.github.com/repos/'+repo+'/actions/runs?event=pull_request&per_page=100&page='+page;
  const resp=await fetchImpl(url,{headers:{
    Authorization:'Bearer '+token,
    Accept:'application/vnd.github+json',
    'X-GitHub-Api-Version':'2022-11-28'
  },signal:AbortSignal.timeout(15000)});
  if(!resp.ok) throw Error('Actions API HTTP '+resp.status);
  const body=await resp.json();
  if(!Array.isArray(body.workflow_runs)) throw Error('Missing workflow_runs in Actions API');
  return body.workflow_runs;
}
async function sampleRecent(token,repo,fetchImpl=fetch,now=Date.now()){
  const after=+now-DAYS*86400000;
  const runs=[];
  let pages=0,truncated=false;
  for(let page=1;page<=MAX_PAGES;page++) {
    const batch=await fetchPage(token,repo,page,fetchImpl);
    pages=page;
    runs.push(...batch);
    if(batch.length<100) break;
    // GitHub returns workflow runs newest first. A full page whose last
    // creation time is older than the window proves later pages irrelevant.
    const oldest=Date.parse(batch.at(-1)?.created_at||'');
    if(Number.isFinite(oldest)&&oldest<after) break;
    if(page===MAX_PAGES) truncated=true;
  }
  return {runs,pages,truncated};
}

const MAX_TIMING_RUNS_PER_WORKFLOW=6;
const TIMING_WORKFLOWS=['Application checks','Docs and installer checks'];

function analyzeJobTimings(entries) {
  const grouped=new Map();
  let completedJobs=0;
  for(const entry of entries) {
    for(const job of entry.jobs) {
      if(job.status!=='completed'||!['success','failure'].includes(job.conclusion)) continue;
      const queue=elapsed(job.created_at,job.started_at);
      const duration=elapsed(job.started_at,job.completed_at);
      if(queue===null||duration===null) continue;
      const name=entry.workflow+' / '+job.name;
      if(!grouped.has(name)) grouped.set(name,{
        workflow:entry.workflow,name:job.name,queue:[],duration:[]
      });
      grouped.get(name).queue.push(queue);
      grouped.get(name).duration.push(duration);
      completedJobs++;
    }
  }
  const rows=[...grouped.values()].map(x=>({
    workflow:x.workflow,name:x.name,n:x.queue.length,
    queueMedian:percentile(x.queue,.5),queueP90:percentile(x.queue,.9),
    durationMedian:percentile(x.duration,.5),durationP90:percentile(x.duration,.9)
  })).sort((a,b)=>b.n-a.n||a.workflow.localeCompare(b.workflow)||a.name.localeCompare(b.name));
  return {sampledRuns:entries.length,completedJobs,rows};
}
async function sampleJobTimings(token,repo,runs,fetchImpl=fetch,now=Date.now()) {
  const after=+now-DAYS*86400000;
  const selected=TIMING_WORKFLOWS.flatMap(name=>runs
    .filter(r=>r.name===name&&r.status==='completed'
      &&['success','failure'].includes(r.conclusion)
      &&Number.isInteger(r.id)&&Date.parse(r.created_at)>=after)
    .sort((a,b)=>Date.parse(b.created_at)-Date.parse(a.created_at))
    .slice(0,MAX_TIMING_RUNS_PER_WORKFLOW));
  const entries=[];
  // Keep API requests bounded and avoid concurrent bursts against GitHub.
  for(const run of selected) {
    const url='https://api.github.com/repos/'+repo+'/actions/runs/'+run.id+'/jobs?per_page=100';
    const resp=await fetchImpl(url,{headers:{
      Authorization:'Bearer '+token,
      Accept:'application/vnd.github+json',
      'X-GitHub-Api-Version':'2022-11-28'
    },signal:AbortSignal.timeout(15000)});
    if(!resp.ok) throw Error('Actions jobs API HTTP '+resp.status);
    const data=await resp.json();
    if(!Array.isArray(data.jobs)||data.total_count>100)
      throw Error('Missing or incomplete Actions jobs page');
    entries.push({workflow:run.name,jobs:data.jobs});
  }
  return analyzeJobTimings(entries);
}
function jobTimingReport(timing) {
  const out=['','### Sampled runner queue wait vs execution','',
    'Up to '+MAX_TIMING_RUNS_PER_WORKFLOW+' recent completed PR runs per workflow; '+
      timing.sampledRuns+' workflow runs and '+timing.completedJobs+' completed jobs sampled.','',
    '| Workflow | Job | n | Queue p50 / p90 (min) | Execution p50 / p90 (min) |',
    '| --- | --- | ---: | ---: | ---: |'];
  for(const row of timing.rows.slice(0,15))
    out.push('| '+row.workflow.replace(/[|\r\n]/g,' ')+' | '+row.name.replace(/[|\r\n]/g,' ')+
      ' | '+row.n+' | '+format(row.queueMedian)+' / '+format(row.queueP90)+
      ' | '+format(row.durationMedian)+' / '+format(row.durationP90)+' |');
  if(!timing.rows.length) out.push('| No eligible completed jobs | — | 0 | n/a | n/a |');
  out.push('','Queue = job.created_at to job.started_at; execution = started_at to completed_at. '+
    'Skipped, cancelled, incomplete and still-queued jobs are excluded. '+
    'This is a bounded recent sample, not an unbiased estimate of the entire week.','');
  return out.join('\n');
}

async function run(env=process.env,fetchImpl=fetch,now=Date.now()){
  const token=env.GH_TOKEN,repo=env.GITHUB_REPOSITORY;
  if(!token||!repo||!/^[\w.-]+\/[\w.-]+$/.test(repo)) throw Error('GH_TOKEN and GITHUB_REPOSITORY are required');
  const sample=await sampleRecent(token,repo,fetchImpl,now);
  const result={...analyze(sample.runs,now),pages:sample.pages,truncated:sample.truncated};
  let summary=report(result);
  if(env.CI_JOB_TIMING_SAMPLE==='1') {
    try {
      summary+=jobTimingReport(await sampleJobTimings(token,repo,sample.runs,fetchImpl,now));
    } catch(err) {
      summary+='\n### Runner queue timing\n\nSampling unavailable: '+err.message+
        '. Existing workflow-run telemetry is unaffected.\n';
    }
  }
  process.stdout.write(summary);
  if(env.GITHUB_STEP_SUMMARY)fs.appendFileSync(env.GITHUB_STEP_SUMMARY,summary+'\n');
  return summary;
}
if(require.main===module)run().catch(e=>{console.error('CI budget telemetry: '+e.message);process.exitCode=1;});
module.exports={percentile,elapsed,analyze,report,fetchPage,sampleRecent,run,MAX_PAGES,
  analyzeJobTimings,sampleJobTimings,jobTimingReport,MAX_TIMING_RUNS_PER_WORKFLOW};
