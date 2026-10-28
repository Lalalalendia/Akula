'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const {validateDocumentStages,validateEvidence,validateVisualReview,hash}=require('../../scripts/canvas-crossapp-validate.cjs');
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const clone=x=>JSON.parse(JSON.stringify(x));
const ids=['754a8ef995f366bc','8132d4d894c80022','7efdbbe0c4742315','59e896bc8da20699','0ba565e7f30e0652'];
function stages(){
  // Use the exact upstream fixture already pinned in #860, not a synthetic lookalike.
  const source=JSON.parse(fs.readFileSync(path.join(__dirname,
    '../../aquilum-app/core/tests/fixtures/jsoncanvas/official-sample.canvas'),'utf8'));
  assert.deepEqual(source.nodes.map(n=>n.id),ids);
  const akula=clone(source);
  akula.nodes.push({id:'akula-new',type:'text',text:'New text',x:0,y:0,width:240,height:120});
  const obsidian=clone(akula);
  obsidian.nodes.unshift({id:'obs-group',type:'group',x:10,y:20,width:420,height:260,label:'Obsidian Group'});
  obsidian.nodes.push({id:'obs-text',type:'text',text:'Obsidian',x:30,y:40,width:180,height:100});
  obsidian.edges.push({id:'obs-edge',fromNode:'obs-text',toNode:'akula-new'});
  return {source,akula,obsidian};
}
test('C5 verifies only precise additive edits and preserves source order',()=>{
  const s=stages();const result=validateDocumentStages(s.source,s.akula,s.obsidian);
  assert.equal(result.akulaAdded,'akula-new');
  assert.deepEqual(result.obsidianAdded,['obs-group','obs-text']);
});
test('C5 rejects Akula mutation of original file path or original edge',()=>{
  const s=stages();s.akula.nodes[2].file='hijack.svg';
  assert.throws(()=>validateDocumentStages(s.source,s.akula,s.obsidian),/original id was modified/);
  const t=stages();t.akula.edges[0].toEnd='none';
  assert.throws(()=>validateDocumentStages(t.source,t.akula,t.obsidian),/original id was modified/);
});
test('C5 rejects Obsidian modification, loss, missing edge, and invented type',()=>{
  const s=stages();s.obsidian.nodes.find(x=>x.id==='akula-new').text='lost';
  assert.throws(()=>validateDocumentStages(s.source,s.akula,s.obsidian),/original id was modified/);
  const t=stages();t.obsidian.edges.pop();
  assert.throws(()=>validateDocumentStages(t.source,t.akula,t.obsidian),/must add a connection/);
  const u=stages();u.obsidian.nodes[0].type='plugin-node';
  assert.throws(()=>validateDocumentStages(u.source,u.akula,u.obsidian),/Expected values to be strictly deep-equal/);
});
test('C5 requires GUI observations, PNG hashes, correct runtime counts and pinned bytes',()=>{
  const s=stages();const dir=fs.mkdtempSync(path.join(os.tmpdir(),'akula-c5-validator-'));
  try{
    const source=Buffer.from(JSON.stringify(s.source)), akula=Buffer.from(JSON.stringify(s.akula)), obsidian=Buffer.from(JSON.stringify(s.obsidian));
    // Signature and plausible byte length only; test proof freshness via SHA-256.
    const png=Buffer.concat([Buffer.from('89504e470d0a1a0a','hex'),Buffer.alloc(1300,0x21)]);
    const obs={};
    for(const [name,app,n,e,bytes] of [
      ['obsidian_initial','obsidian',5,1,source],['akula_edited','akula',6,1,akula],
      ['obsidian_reopened','obsidian',6,1,akula],['obsidian_edited','obsidian',8,2,obsidian],
      ['akula_reopened','akula',8,2,obsidian]]){
      const image=name+'.png';fs.writeFileSync(path.join(dir,image),png);
      obs[name]={app,nodes:n,edges:e,sha256:hash(bytes),screenshot:image,screenshotSha256:hash(png),
        ...(app==='obsidian'?{viewType:'canvas',path:'reference.canvas',domNodes:n}:
          {visibleCanvas:true,accessibilityLabel:'Canvas: '+n+' nodes, '+e+' edges'})};
    }
    const report={versions:{obsidian:'1.14.5',akulaBinarySha256:'a'.repeat(64)},os:'Windows 11 x64',observations:obs};
    assert.doesNotThrow(()=>validateEvidence(dir,source,akula,obsidian,source,report));
    report.observations.obsidian_reopened.domNodes=5;
    assert.throws(()=>validateEvidence(dir,source,akula,obsidian,source,report),/missing rendered DOM nodes/);
    report.observations.obsidian_reopened.domNodes=6;
    report.observations.akula_reopened.screenshotSha256='0'.repeat(64);
    assert.throws(()=>validateEvidence(dir,source,akula,obsidian,source,report),/PNG hash mismatch/);
    report.observations.akula_reopened.screenshotSha256=hash(png);
    assert.throws(()=>validateEvidence(dir,source,akula,obsidian,Buffer.from('not official'),report),/reference bytes must be exactly pinned/);
  }finally{fs.rmSync(dir,{force:true,recursive:true});}
});


test('C5 Windows collector keeps a single GUI flow and fail-closed disk/CLI guards',()=>{
  const source=fs.readFileSync(path.join(__dirname,
    '../../scripts/windows-canvas-obsidian-crossapp.ps1'),'utf8');
  for(const name of [
    'Invoke-Obsidian','Get-ObsidianCanvas','Capture-Obsidian','Get-AkulaCanvas',
    'Capture-Akula','Wait-CanvasOnDisk','Close-Akula']){
    assert.equal(source.split('function '+name).length-1,1,
      'duplicated or missing PowerShell collector function '+name);
  }
  assert.match(source,/AKULA_C5_STATE_/,'CLI eval must use per-call marker');
  assert.match(source,/StartsWith\(\$marker/,'CLI response must be correlated');
  assert.doesNotMatch(source,/console\.log\(result\)/,'CLI eval result must not depend on console output');
  assert.match(source,/if \(!\$AlreadyOpen\)/,'do not reopen Obsidian during operator edit');
  assert.match(source,/Capture-Obsidian 'obsidian_edited' 8 2 -AlreadyOpen/);
  assert.match(source,/Wait-CanvasOnDisk 6 1/);
  assert.match(source,/Wait-CanvasOnDisk 8 2/);
  assert.match(source,/AKULA-C5-DISPOSABLE-ONLY/);
  assert.match(source,/Reference canvas is not pristine/);
  assert.match(source,/Strict cross-app evidence validation FAILED/);
  assert.ok(source.trimEnd().endsWith('}'),'collector ends at the sole main try/finally');
});


test('C5 emits cross-app completion only after SHA-bound five-screenshot human review',()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'akula-c5-review-'));
  try{
    const phases=['obsidian_initial','akula_edited','obsidian_reopened','obsidian_edited','akula_reopened'];
    const report={observations:Object.fromEntries(phases.map((name,i)=>[name,{screenshotSha256:(i+1).toString(16).repeat(64)}]))};
    assert.equal(validateVisualReview(dir,report),null,'absence of human review is not acceptance');
    const review={
      reviewer:'GUI acceptance operator',
      reviewedAtUtc:'2026-10-28T12:00:00Z',
      phases:Object.fromEntries(phases.map((name,i)=>[name,{
        screenshotSha256:(i+1).toString(16).repeat(64),visualPass:true
      }])),
      checks:{readableText:true,visibleObjects:true,visibleConnections:true,
        groupLayering:true,filePlaceholders:true}
    };
    const file=path.join(dir,'visual-review.json');
    fs.writeFileSync(file,JSON.stringify(review));
    assert.equal(validateVisualReview(dir,report).reviewer,'GUI acceptance operator');
    review.phases.obsidian_reopened.screenshotSha256='0'.repeat(64);
    fs.writeFileSync(file,JSON.stringify(review));
    assert.throws(()=>validateVisualReview(dir,report),/different screenshot/,'stale visual review refused');
    review.phases.obsidian_reopened.screenshotSha256='3'.repeat(64);
    review.checks.visibleConnections=false;
    fs.writeFileSync(file,JSON.stringify(review));
    assert.throws(()=>validateVisualReview(dir,report),/visual acceptance missing visibleConnections/);
    review.checks.visibleConnections=true;
    delete review.phases.akula_reopened;
    fs.writeFileSync(file,JSON.stringify(review));
    assert.throws(()=>validateVisualReview(dir,report),/visual review phases incomplete/);
  }finally{fs.rmSync(dir,{recursive:true,force:true});}
});
