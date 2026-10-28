'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const {validateDocumentStages,validateEvidence,hash}=require('../../scripts/canvas-crossapp-validate.cjs');
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
