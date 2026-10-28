'use strict';
// AK-DOC-C5: verify artifacts from two real GUI applications, not just JSON parsers.
// This validator alone does NOT attest that either GUI was executed. The Windows
// collector provides bounded GUI observations, version and screenshot evidence.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');
const {isDeepStrictEqual} = require('node:util');

const NODES = ['754a8ef995f366bc','8132d4d894c80022','7efdbbe0c4742315',
  '59e896bc8da20699','0ba565e7f30e0652'];
const EDGE = '6fa11ab87f90b8af';
const PHASES = [
  ['obsidian_initial','obsidian',5,1],
  ['akula_edited','akula',6,1],
  ['obsidian_reopened','obsidian',6,1],
  ['obsidian_edited','obsidian',8,2],
  ['akula_reopened','akula',8,2],
];
function hash(bytes) { return crypto.createHash('sha256').update(bytes).digest('hex'); }
function load(file) { return fs.readFileSync(file); }
function json(file) { return JSON.parse(load(file).toString('utf8')); }
function ids(objects, title) {
  assert.ok(Array.isArray(objects), title+' must be array');
  const result = new Map();
  for(const o of objects){
    assert.ok(o&&typeof o==='object'&&!Array.isArray(o)&&typeof o.id==='string'&&o.id.length,title+' id invalid');
    assert.ok(!result.has(o.id),title+' duplicate id '+o.id);
    result.set(o.id,o);
  }
  return result;
}
function requirePreserved(before, after, type) {
  const prev = ids(before,type+' baseline');
  const next = ids(after,type+' output');
  const beforeOrder = before.map(o=>o.id);
  const stillInOrder = after.filter(o=>prev.has(o.id)).map(o=>o.id);
  assert.deepEqual(stillInOrder,beforeOrder,type+' original source order changed');
  for(const [id,value] of prev){
    assert.ok(next.has(id),type+' original id was removed: '+id);
    assert.ok(isDeepStrictEqual(next.get(id),value),type+' original id was modified: '+id);
  }
  return {prev,next,added:after.filter(o=>!prev.has(o.id))};
}
function requireNewGeometry(node,title) {
  for(const key of ['x','y','width','height']){
    assert.ok(typeof node[key]==='number'&&Number.isFinite(node[key]),title+' invalid '+key);
  }
  assert.ok(node.width>0&&node.height>0,title+' invalid size');
  if(node.type==='text') assert.ok(typeof node.text==='string'&&node.text.length,title+' missing text');
}
function validateDocumentStages(source,akula,obsidian) {
  assert.equal(source.nodes?.length,5,'official reference node count');
  assert.equal(source.edges?.length,1,'official reference edge count');
  assert.deepEqual(source.nodes.map(x=>x.id),NODES,'must begin with pinned official sample');
  assert.equal(source.edges[0].id,EDGE,'official edge mismatch');
  assert.equal(source.nodes[2].file,'_site/logo.svg','official file reference mismatch');
  assert.equal(source.nodes[3].type,'text','official text card mismatch');
  assert.equal(source.edges[0].fromNode,NODES[2]);
  assert.equal(source.edges[0].toNode,NODES[3]);
  assert.deepEqual(Object.keys(source).sort(),['edges','nodes'],'official reference root keys mismatch');

  assert.equal(akula.nodes?.length,6,'Akula must add exactly one node through its GUI');
  assert.equal(akula.edges?.length,1,'Akula must keep the original edge');
  const aNodes=requirePreserved(source.nodes,akula.nodes,'Akula nodes');
  requirePreserved(source.edges,akula.edges,'Akula edges');
  assert.equal(aNodes.added.length,1);
  assert.equal(aNodes.added[0].type,'text','Akula new node must be a text card');
  requireNewGeometry(aNodes.added[0],'Akula added text');
  assert.equal(obsidian.nodes?.length,8,'Obsidian GUI must add one group and one text card');
  assert.equal(obsidian.edges?.length,2,'Obsidian GUI must add a connection');
  const oNodes=requirePreserved(akula.nodes,obsidian.nodes,'Obsidian nodes');
  const oEdges=requirePreserved(akula.edges,obsidian.edges,'Obsidian edges');
  assert.deepEqual(oNodes.added.map(x=>x.type).sort(),['group','text']);
  for(const node of oNodes.added) requireNewGeometry(node,'Obsidian added '+node.type);
  assert.equal(oEdges.added.length,1,'exactly one Obsidian edge required');
  const e=oEdges.added[0];
  assert.ok(oNodes.next.has(e.fromNode)&&oNodes.next.has(e.toNode),'new edge endpoints must exist');
  const addedIds=new Set(oNodes.added.map(x=>x.id));
  assert.ok(addedIds.has(e.fromNode)||addedIds.has(e.toNode),'new edge must connect a newly created object');
  for(const [k,v] of Object.entries(source))if(k!=='nodes'&&k!=='edges'){
    assert.ok(isDeepStrictEqual(akula[k],v)&&isDeepStrictEqual(obsidian[k],v),'root extension '+k+' lost');
  }
  for(const doc of [akula,obsidian]){
    assert.deepEqual(Object.keys(doc).sort(),Object.keys(source).sort(),'root keys unexpectedly added/dropped');
  }
  return {akulaAdded:aNodes.added[0].id,obsidianAdded:oNodes.added.map(n=>n.id),newEdge:e.id};
}
function ensurePng(dir,file) {
  assert.ok(typeof file==='string'&&/^[a-z0-9_-]+\.png$/i.test(file),'invalid PNG evidence path');
  const bytes=load(path.join(dir,file));
  assert.ok(bytes.length>1024,'empty GUI screenshot '+file);
  assert.equal(bytes.subarray(0,8).toString('hex'),'89504e470d0a1a0a','invalid PNG header '+file);
  return hash(bytes);
}
function validateEvidence(dir,sourceBytes,akulaBytes,obsidianBytes,refBytes,report) {
  const versions=report.versions||{};
  assert.match(versions.obsidian||'',/\b\d+\.\d+\.\d+\b/,'Obsidian version missing');
  assert.match(versions.akulaBinarySha256||'',/^[a-f0-9]{64}$/i,'installed Akula binary hash missing');
  assert.ok(typeof report.os==='string'&&report.os.length>6,'OS evidence missing');
  assert.equal(hash(sourceBytes),hash(refBytes),'reference bytes must be exactly pinned');
  assert.notEqual(hash(akulaBytes),hash(sourceBytes),'Akula did not write an edited revision');
  assert.notEqual(hash(obsidianBytes),hash(akulaBytes),'Obsidian did not write an edited revision');
  assert.ok(report.observations&&typeof report.observations==='object','GUI observations missing');
  const shaByPhase={obsidian_initial:hash(sourceBytes),akula_edited:hash(akulaBytes),
    obsidian_reopened:hash(akulaBytes),obsidian_edited:hash(obsidianBytes),akula_reopened:hash(obsidianBytes)};
  for(const [phase,app,expectedNodes,expectedEdges] of PHASES){
    const o=report.observations[phase];
    assert.ok(o&&o.app===app,'missing GUI observation '+phase);
    assert.equal(o.nodes,expectedNodes,phase+' runtime node count mismatch');
    assert.equal(o.edges,expectedEdges,phase+' runtime edge count mismatch');
    assert.equal((o.sha256||'').toLowerCase(),shaByPhase[phase],phase+' file hash mismatch');
    assert.equal(o.screenshotSha256?.toLowerCase(),ensurePng(dir,o.screenshot),phase+' PNG hash mismatch');
    if(app==='obsidian'){
      assert.equal(o.viewType,'canvas',phase+' was not a native Obsidian Canvas view');
      assert.equal(o.path,'reference.canvas',phase+' opened wrong file');
      assert.equal(o.domNodes,expectedNodes,phase+' missing rendered DOM nodes');
    }else{
      assert.equal(o.accessibilityLabel,'Canvas: '+expectedNodes+' nodes, '+expectedEdges+' edges',phase+' UIA mismatch');
      assert.ok(o.visibleCanvas===true,phase+' viewport was not visibly laid out');
    }
  }
}
// Machine-readable GUI presence is necessary, not a visual review.
// An optional human attestation is bound to the exact five screenshot hashes.
// Without it the CLI emits MACHINE_EVIDENCE_PASS, never CROSSAPP_PASS.
function validateVisualReview(evidenceDir,report) {
  const file=path.join(path.resolve(evidenceDir),'visual-review.json');
  if(!fs.existsSync(file))return null;
  const review=json(file);
  assert.ok(review && typeof review==='object'&&!Array.isArray(review),'visual review must be object');
  assert.ok(typeof review.reviewer==='string'&&review.reviewer.trim().length>=3,'visual reviewer missing');
  assert.ok(typeof review.reviewedAtUtc==='string'&&!Number.isNaN(Date.parse(review.reviewedAtUtc)),'visual review timestamp missing');
  assert.ok(review.phases&&typeof review.phases==='object'&&!Array.isArray(review.phases),'visual phase reviews missing');
  assert.deepEqual(Object.keys(review.phases).sort(),PHASES.map(x=>x[0]).sort(),'visual review phases incomplete');
  for(const [phase] of PHASES){
    const item=review.phases[phase];
    assert.ok(item&&typeof item==='object'&&!Array.isArray(item),'missing review '+phase);
    assert.equal(item.screenshotSha256?.toLowerCase(),report.observations[phase].screenshotSha256.toLowerCase(),
      'review bound to a different screenshot '+phase);
    assert.equal(item.visualPass,true,'screenshot not visually accepted '+phase);
  }
  for(const key of ['readableText','visibleObjects','visibleConnections','groupLayering','filePlaceholders']){
    assert.equal(review.checks?.[key],true,'visual acceptance missing '+key);
  }
  return {reviewer:review.reviewer,reviewedAtUtc:review.reviewedAtUtc};
}

function validateFiles(evidenceDir,pinnedPath) {
  const root=path.resolve(evidenceDir);
  const sourceBytes=load(path.join(root,'00-original.canvas'));
  const akulaBytes=load(path.join(root,'01-akula.canvas'));
  const obsidianBytes=load(path.join(root,'02-obsidian.canvas'));
  const report=json(path.join(root,'report.json'));
  const source=JSON.parse(sourceBytes.toString('utf8'));
  const akula=JSON.parse(akulaBytes.toString('utf8'));
  const obsidian=JSON.parse(obsidianBytes.toString('utf8'));
  const result=validateDocumentStages(source,akula,obsidian);
  validateEvidence(root,sourceBytes,akulaBytes,obsidianBytes,load(pinnedPath),report);
  return {...result,obsidianVersion:report.versions.obsidian,sourceSha256:hash(sourceBytes),
    akulaSha256:hash(akulaBytes),obsidianSha256:hash(obsidianBytes),
    screenshotCount:PHASES.length};
}
if(require.main===module){
  try{
    if(process.argv.length!==4) throw Error('usage: node scripts/canvas-crossapp-validate.cjs <evidence-dir> <pinned-fixture>');
    const proof=validateFiles(process.argv[2],process.argv[3]);
    const report=json(path.join(path.resolve(process.argv[2]),'report.json'));
    const visual=validateVisualReview(process.argv[2],report);
    if(visual)process.stdout.write('AKULA_C5_OBSIDIAN_CROSSAPP_PASS '+JSON.stringify({...proof,visual})+'\n');
    else process.stdout.write('AKULA_C5_OBSIDIAN_MACHINE_EVIDENCE_PASS '+JSON.stringify(proof)+'\n'+
      'AKULA_C5_OBSIDIAN_VISUAL_REVIEW_PENDING add a SHA-bound visual-review.json after inspecting all five PNGs\n');
  }catch(e){console.error('AKULA_C5_OBSIDIAN_CROSSAPP_FAIL '+e.message);process.exitCode=1;}
}
module.exports={hash,validateDocumentStages,validateEvidence,validateFiles,validateVisualReview};
