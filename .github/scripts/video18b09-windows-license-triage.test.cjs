'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {buildTriage,TARGET,EXECUTABLES}=require('./video18b09-windows-license-triage.cjs');
const repo=path.resolve(__dirname,'../..');
const workflow=fs.readFileSync(path.join(repo,'.github/workflows/akula-video18b09-windows-license-triage.yml'),'utf8');
const allow=JSON.parse(fs.readFileSync(path.join(repo,'docs/release/video18b04-five-exe-allowlist.json'),'utf8'));
const head='a'.repeat(40);
function sample(){
  const shared={id:'shared@1',name:'shared',version:'1.0',source:'registry+https://github.com/rust-lang/crates.io-index'};
  const extra={id:'optional@1',name:'optional',version:'1.0',source:'registry+https://github.com/rust-lang/crates.io-index'};
  const names={akula:'aquilum-native',mux:'akula-video-lite-lab',shiva:'cli'};
  const filtered={},roots={},unique=new Map();
  for(const [key,name] of Object.entries(names)){
    const root={id:name+'@1',name,version:'1.0',source:null};
    filtered[key]={packages:[root,shared],resolve:{nodes:[{id:root.id,deps:[{pkg:shared.id}]},{id:shared.id,deps:[]}]}};
    const members=[root,shared,extra];
    roots[key]={root:name,packages:members.map(p=>({name:p.name,version:p.version,source:p.source||'local-source'}))};
    for(const p of members)unique.set(p.name+'|'+p.version+'|'+(p.source||'local-source'),{name:p.name,version:p.version,source:p.source||'local-source'});
  }
  const packages=[...unique.values()];
  const full={exact_akula_head:head,release_admitted:false,legal_approval:false,shiva_source_pin:'b'.repeat(40),roots,unique_packages:packages};
  const evidence={source_head:head,release_admitted:false,legal_approval:false,license_text_review_complete:false,
    package_count:packages.length,packages:packages.map(p=>({...p,license_expression:'MIT',status:p.name==='shared'?'TEXT_NOT_VERIFIED':'TEXT_FOUND_REQUIRES_REVIEW'}))};
  return {filtered,full,evidence,allow:structuredClone(allow)};
}
test('VIDEO18B09 reports only target-reachable Windows closure and keeps release HOLD',()=>{
 const a=sample();const r=buildTriage(a.filtered,a.full,a.evidence,a.allow,head);
 assert.equal(r.target,TARGET);
 assert.equal(r.target_unique_packages,4);
 assert.equal(r.conservative_unique_packages,5);
 assert.equal(r.excluded_from_conservative,1);
 assert.equal(r.license_text_not_verified,1);
 assert.equal(r.priority_review_queue[0].review_priority,'RUNTIME_COMPONENT_DEPENDENCY');
 assert.deepEqual(r.priority_review_queue[0].roots,['akula','mux','shiva']);
 assert.equal(r.release_admitted,false);
 assert.equal(r.legal_approval,false);
 assert.equal(r.license_text_review_complete,false);
 assert.equal(r.bundled_executables.length,5);
});
test('VIDEO18B09 rejects release promotion, mismatched SHA, wrong five executable bundle',()=>{
 const a=sample();a.full.release_admitted=true;
 assert.throws(()=>buildTriage(a.filtered,a.full,a.evidence,a.allow,head),/HOLD/);
 const b=sample();b.evidence.source_head='f'.repeat(40);
 assert.throws(()=>buildTriage(b.filtered,b.full,b.evidence,b.allow,head),/HOLD/);
 const c=sample();c.allow.components.push({...c.allow.components[0],name:'sixth.exe'});
 assert.throws(()=>buildTriage(c.filtered,c.full,c.evidence,c.allow,head),/HOLD/);
});
test('VIDEO18B09 refuses missing graph, unmatched package, or missing license evidence',()=>{
 const a=sample();a.filtered.mux.resolve.nodes.pop();
 assert.throws(()=>buildTriage(a.filtered,a.full,a.evidence,a.allow,head),/missing package|unresolved/i);
 const b=sample();b.filtered.mux.packages[1]={...b.filtered.mux.packages[1],name:'unlisted'};
 assert.throws(()=>buildTriage(b.filtered,b.full,b.evidence,b.allow,head),/unlisted|not in pinned/i);
 const c=sample();c.evidence.packages.pop();c.evidence.package_count--;
 assert.throws(()=>buildTriage(c.filtered,c.full,c.evidence,c.allow,head),/mismatch/);
});
test('VIDEO18B09 exact HEAD feature-gated Windows metadata CI remains PR-only and read-only',()=>{
 const h=workflow.split(/\r?\npermissions:/,1)[0];
 assert.match(h,/^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
 for(const p of ['.github/scripts/video18b09-windows-license-triage.cjs',
  '.github/scripts/video18b09-windows-license-triage.test.cjs',
  '.github/workflows/akula-video18b09-windows-license-triage.yml',
  'docs/release/video18b09-windows-license-triage.md',
  'docs/release/video18b04-five-exe-allowlist.json']){
   assert.ok(h.includes("      - '"+p+"'"),p);
 }
 assert.match(workflow,/contents: read/);
 assert.match(workflow,/persist-credentials: false/);
 assert.match(workflow,/ref: cb542d30d2c18a18b1ee457f5158d5b0f551757e/);
 assert.match(workflow,/--filter-platform x86_64-pc-windows-msvc/g);
 assert.match(workflow,/--features installed,video-rust-bundle/);
 assert.match(workflow,/cargo metadata --locked --all-features/g);
 assert.match(workflow,/retention-days: 1/);
 assert.ok(!workflow.includes('workflow_dispatch:'));
 assert.ok(!/gh release create|cargo packager|publish_beta/i.test(workflow));
 assert.deepEqual(allow.components.map(c=>c.name),EXECUTABLES);
});
