'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const {fileEvidence,inspectPackage,buildEvidence}=require('./video18b08-license-file-evidence.cjs');
const root=path.resolve(__dirname,'../..');
const workflow=fs.readFileSync(path.join(root,'.github/workflows/akula-video18b08-license-file-evidence.yml'),'utf8');
const allow=JSON.parse(fs.readFileSync(path.join(root,'docs/release/video18b04-five-exe-allowlist.json'),'utf8'));
const head='a'.repeat(40);
function fixture(){
 const dir=fs.mkdtempSync(path.join(os.tmpdir(),'video18b08-'));
 const one=path.join(dir,'one');fs.mkdirSync(one);
 fs.writeFileSync(path.join(one,'Cargo.toml'),'[package]\nname="dummy"\n');
 fs.writeFileSync(path.join(one,'LICENSE'),'original terms');
 const pkg={id:'dummy 1.0.0',name:'dummy',version:'1.0.0',source:'local-source',manifest_path:path.join(one,'Cargo.toml'),license:'MIT'};
 const metas={};
 for(const [key,name] of [['akula','aquilum-native'],['mux','akula-video-lite-lab'],['shiva','cli']]){
   const rootPkg={...pkg,id:name+' 1.0.0',name};
   metas[key]={packages:[rootPkg],resolve:{nodes:[{id:rootPkg.id,deps:[]}]}};
 }
 const inventory={release_admitted:false,legal_approval:false,exact_akula_head:head,unique_packages:Object.values(metas).map(x=>({name:x.packages[0].name,version:x.packages[0].version,source:x.packages[0].source}))};
 return {dir,one,pkg,metas,inventory};
}
test('VIDEO18B08 records an actual license file by digest without auto approval',t=>{
 const f=fixture();t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
 const e=inspectPackage(f.pkg);
 assert.equal(e.status,'TEXT_FOUND_REQUIRES_REVIEW');
 assert.equal(e.files[0].status,'PRESENT_UNREVIEWED');
 assert.match(e.files[0].sha256,/^[a-f0-9]{64}$/);
 const r=buildEvidence(f.metas,f.inventory,allow,head);
 assert.equal(r.package_count,3);
 assert.equal(r.release_admitted,false);
 assert.equal(r.legal_approval,false);
 assert.equal(r.license_text_review_complete,false);
});
test('VIDEO18B08 refuses path escape, missing file, oversized file and symlinks',t=>{
 const f=fixture();t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
 assert.equal(fileEvidence(f.one,'../Cargo.toml').status,'BLOCKED_UNSAFE_PATH');
 assert.equal(fileEvidence(f.one,'missing').status,'MISSING');
 fs.writeFileSync(path.join(f.one,'empty'),'');
 assert.equal(fileEvidence(f.one,'empty').status,'BLOCKED_INVALID_SIZE');
 fs.writeFileSync(path.join(f.one,'huge'),Buffer.alloc(256*1024+1));
 assert.equal(fileEvidence(f.one,'huge').status,'BLOCKED_INVALID_SIZE');
 fs.symlinkSync(path.join(f.one,'LICENSE'),path.join(f.one,'linked'));
 assert.equal(fileEvidence(f.one,'linked').status,'BLOCKED_NOT_REGULAR_FILE');
});
test('VIDEO18B08 rejects missing inventory closure, false approval, and incorrect SPDX',t=>{
 const f=fixture();t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
 assert.throws(()=>buildEvidence(f.metas,{...f.inventory,unique_packages:[]},allow,head),/mismatch/);
 assert.throws(()=>buildEvidence(f.metas,{...f.inventory,release_admitted:true},allow,head),/HOLD/);
 const changed=structuredClone(allow);changed.components[2].license='unknown';
 assert.throws(()=>buildEvidence(f.metas,f.inventory,changed,head),/HOLD/);
});
test('VIDEO18B08 workflow is exact-head, read-only and retains HOLD',()=>{
 const header=workflow.split(/\r?\npermissions:/,1)[0];
 assert.match(header,/^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
 for(const f of ['.github/scripts/video18b08-license-file-evidence.cjs','.github/scripts/video18b08-license-file-evidence.test.cjs','.github/workflows/akula-video18b08-license-file-evidence.yml','docs/release/video18b04-five-exe-allowlist.json'])assert.ok(header.includes("'"+f+"'"),f);
 assert.match(workflow,/contents: read/);
 assert.match(workflow,/persist-credentials: false/);
 assert.match(workflow,/retention-days: 1/);
 assert.match(workflow,/ref: cb542d30d2c18a18b1ee457f5158d5b0f551757e/);
 assert.match(workflow,/cargo metadata --locked --all-features/g);
 assert.ok(!workflow.includes('workflow_dispatch:'));
 assert.ok(!/gh release create|cargo packager|publish_beta/i.test(workflow));
});
