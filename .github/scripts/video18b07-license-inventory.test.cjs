'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {test} = require('node:test');
const {reachablePackages,componentPackage,makeInventory} = require('./video18b07-license-inventory.cjs');
const repo = path.resolve(__dirname,'../..');
const workflow = fs.readFileSync(path.join(repo,'.github/workflows/akula-video18b07-license-inventory.yml'),'utf8');
const files = [
  '.github/scripts/video18b07-license-inventory.cjs',
  '.github/scripts/video18b07-license-inventory.test.cjs',
  '.github/workflows/akula-video18b07-license-inventory.yml',
  'docs/release/video18b07-license-inventory.md',
  'docs/release/video18b04-five-exe-allowlist.json',
];
function sample(name) {
  const root = {id:name+' 1.0.0',name,version:'1.0.0',source:null,license:'AGPL-3.0-only'};
  const dep = {id:'example 0.1.0',name:'example',version:'0.1.0',source:'registry+https://github.com/rust-lang/crates.io-index',license:'MIT OR Apache-2.0'};
  return {packages:[root,dep],resolve:{nodes:[
    {id:root.id,deps:[{pkg:dep.id}]},
    {id:dep.id,deps:[]},
  ]}};
}
function allowance() {
  const names=['akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe'];
  return {release_admitted:false,components:names.map(n => ({
    name:n,kind:n==='akula-yt-dlp.exe'||n==='akula-quickjs.exe'?'pinned-upstream':'source-built',
    license_status:'REVIEW_NOTICE_PACKAGING',sha256:'a'.repeat(64),license:'MIT',
  }))};
}
const sampleAll = () => ({akula:sample('aquilum-native'),mux:sample('akula-video-lite-lab'),shiva:sample('cli')});
const head = 'f'.repeat(40);
test('VIDEO18B07 full dependency closure and distinct license graph, still HOLD', () => {
  const result=makeInventory(sampleAll(),allowance(),head);
  assert.equal(result.release_admitted,false);
  assert.equal(result.legal_approval,false);
  assert.equal(result.sbom_is_license_attestation,false);
  assert.equal(result.roots.akula.count,2);
  assert.equal(result.roots.mux.count,2);
  assert.equal(result.roots.shiva.count,2);
  assert.equal(result.package_count_deduplicated,4);
  assert.equal(result.missing_license_metadata.length,0);
  assert.equal(result.bundled_upstream_executables.length,2);
});
test('VIDEO18B07 rejects missing graph nodes instead of silently truncating dependencies',()=>{
  const m=sample('cli');m.resolve.nodes.pop();
  assert.throws(()=>reachablePackages(m,'cli'),/VIDEO18B07/);
});
test('VIDEO18B07 detects missing transitive license metadata and does not approve it',()=>{
  const m=sampleAll();m.mux.packages[1].license=null;
  const r=makeInventory(m,allowance(),head);
  assert.equal(componentPackage(m.mux.packages[1]).review_status,'BLOCKED_MISSING_LICENSE_METADATA');
  assert.equal(r.release_admitted,false);
});
test('VIDEO18B07 rejects removed release HOLD',()=>{
  const a=allowance();a.release_admitted=true;
  assert.throws(()=>makeInventory(sampleAll(),a,head),/VIDEO18B07/);
});
test('VIDEO18B07 rejects incomplete five EXE allowlist',()=>{
  const a=allowance();a.components.pop();
  assert.throws(()=>makeInventory(sampleAll(),a,head),/VIDEO18B07/);
});
test('VIDEO18B07 rejects malformed source commit',()=>{
  assert.throws(()=>makeInventory(sampleAll(),allowance(),'main'),/VIDEO18B07/);
});
test('VIDEO18B07 workflow is exact-head and cannot publish code or unsigned binaries',()=>{
  const header=workflow.split(/\r?\npermissions:/,1)[0];
  assert.match(header,/^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
  for(const f of files)assert.ok(header.includes("      - '"+f+"'"),'VIDEO18B07 missing source trigger '+f);
  assert.match(workflow,/concurrency:\s*\n  group:/);
  assert.match(workflow,/persist-credentials: false/);
  assert.match(workflow,/permissions:\s*\n  contents: read/);
  assert.ok(workflow.includes('ref: cb542d30d2c18a18b1ee457f5158d5b0f551757e'));
  assert.ok(workflow.includes('ref: $' + '{{ github.event.pull_request.head.sha }}'));
  assert.equal((workflow.match(/cargo metadata --locked --all-features/g)||[]).length,3);
  assert.match(workflow,/retention-days: 1/);
  assert.ok(!workflow.includes('workflow_dispatch:'));
  assert.ok(!/gh release create|cargo packager|publish_beta|upload-artifact[\s\S]*\.exe/i.test(workflow));
});
