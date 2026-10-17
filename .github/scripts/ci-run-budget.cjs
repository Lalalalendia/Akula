'use strict';
const fs=require('node:fs');
function percentile(nums,fraction) {
  const sorted=nums.filter(Number.isFinite).sort((a,b)=>a-b);
  return sorted.length?sorted[Math.max(0,Math.ceil(sorted.length*fraction)-1)]:null;
}
function elapsed(a,b) {
  const start=Date.parse(a||''),end=Date.parse(b||'');
  return Number.isFinite(start)&&Number.isFinite(end)&&end>=start?(end-start)/60000:null;
}
function analyze(runs,now=Date.now()) {
  const after=+now-7*86400000;
  const recent=runs.filter(r=>r.event==='pull_request'&&Date.parse(r.created_at)>=after);
  const bySha=new Map(), durations=new Map();
  let success=0,fail=0,cancelled=0;
  for(const r of recent) {
    if(r.head_branch&&r.head_sha) {
      const key=r.head_branch+'|'+r.head_sha;
      if(!bySha.has(key)) bySha.set(key,{workflows:new Set(),runs:0});
      const group=bySha.get(key);
      group.workflows.add(r.name);group.runs++;
    }
    if(r.status!=='completed') continue;
    if(r.conclusion==='success') success++;
    if(r.conclusion==='failure') fail++;
    if(r.conclusion==='cancelled') cancelled++;
    if(!['success','failure'].includes(r.conclusion)) continue;
    const mins=elapsed(r.created_at,r.updated_at);
    if(mins===null) continue;
    if(!durations.has(r.name)) durations.set(r.name,[]);
    durations.get(r.name).push(mins);
  }
  const gate=durations.get('Application checks')||[];
  const fanout=[...bySha.values()].map(g=>g.workflows.size);
  const duplicates=[...bySha.values()].map(g=>g.runs);
  const budgets=[];
  if(gate.length>=10&&percentile(gate,.9)>10) budgets.push('Main PR gate p90 exceeds 10 minutes');
  if(gate.length>=10&&percentile(gate,.5)>8) budgets.push('Main PR gate median exceeds 8 minutes');
  if(fanout.length>=10&&percentile(fanout,.9)>3) budgets.push('PR workflow fan-out p90 exceeds 3');
  return {recent:recent.length,commits:bySha.size,success,fail,cancelled,
    gate:{n:gate.length,median:percentile(gate,.5),p90:percentile(gate,.9)},
    fanout:{median:percentile(fanout,.5),p90:percentile(fanout,.9),max:fanout.length?Math.max(...fanout):null},
    runsPerCommit:{median:percentile(duplicates,.5),p90:percentile(duplicates,.9)},
    workflows:[...durations.entries()].map(([name,arr])=>({name,n:arr.length,median:percentile(arr,.5),p90:percentile(arr,.9)}))
      .sort((a,b)=>b.n-a.n).slice(0,12),budgets};
}
function format(n){return n==null?'n/a':n.toFixed(1);}
function report(d){
  const out=['## Weekly CI / fan-out budget (diagnostic)','',
    'Sample: last 7 days, at most 200 most recent PR workflow runs; '+d.recent+' runs; '+d.commits+' commit+branch pairs.','',
    '- Main PR gate: n='+d.gate.n+', median '+format(d.gate.median)+' min, p90 '+format(d.gate.p90)+' min.',
    '- Unique workflows per PR SHA: median '+format(d.fanout.median)+', p90 '+format(d.fanout.p90)+', max '+format(d.fanout.max)+'.',
    '- Raw runs per PR SHA, including repeated checks: median '+format(d.runsPerCommit.median)+', p90 '+format(d.runsPerCommit.p90)+'.',
    '- Completed: '+d.success+' success, '+d.fail+' failure, '+d.cancelled+' cancelled.','',
    '| Workflow | Samples | Median minutes | p90 minutes |',
    '| --- | ---: | ---: | ---: |'];
  for(const w of d.workflows) out.push('| '+w.name.replace(/[|\r\n]/g,' ')+' | '+w.n+' | '+format(w.median)+' | '+format(w.p90)+' |');
  out.push('','### Advisory warnings');
  if(d.budgets.length) for(const warning of d.budgets) out.push('- '+warning);
  else out.push('None in this sample.');
  out.push('','Durations are run-created-to-updated wall time (including possible queue delay), NOT billed runner time. Jobs may run concurrently. Sample is truncated at 200, so interpret trends only after accumulating data. No budget warning blocks merging.','');
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
async function run(env=process.env,fetchImpl=fetch){
  const token=env.GH_TOKEN,repo=env.GITHUB_REPOSITORY;
  if(!token||!repo||!/^[\w.-]+\/[\w.-]+$/.test(repo)) throw Error('GH_TOKEN and GITHUB_REPOSITORY are required');
  const first=await fetchPage(token,repo,1,fetchImpl);
  const second=first.length===100?await fetchPage(token,repo,2,fetchImpl):[];
  const summary=report(analyze([...first,...second]));
  process.stdout.write(summary);
  if(env.GITHUB_STEP_SUMMARY)fs.appendFileSync(env.GITHUB_STEP_SUMMARY,summary+'\n');
  return summary;
}
if(require.main===module)run().catch(e=>{console.error('CI budget telemetry: '+e.message);process.exitCode=1;});
module.exports={percentile,elapsed,analyze,report,fetchPage,run};
