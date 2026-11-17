'use strict';
const fs=require('node:fs');
const path=require('node:path');
const crypto=require('node:crypto');
const {reachablePackages}=require('./video18b07-license-inventory.cjs');
const MAX_BYTES=256*1024;
const CANDIDATES=['LICENSE','LICENSE-MIT','LICENSE-APACHE','LICENSE-APACHE-2.0','LICENSE.md','LICENSE.txt','LICENCE','LICENCE.md','COPYING','COPYING.md','NOTICE','NOTICE.md','NOTICE.txt','UNLICENSE'];
function fileEvidence(root,relative){
  if(typeof relative!=='string'||!relative||path.isAbsolute(relative)||relative.split(/[\\/]/).includes('..')||relative.includes('\0'))return {status:'BLOCKED_UNSAFE_PATH',file:relative};
  const base=path.resolve(root),file=path.resolve(base,relative);
  if(file===base||!file.startsWith(base+path.sep))return {status:'BLOCKED_UNSAFE_PATH',file:relative};
  try{
    const realRoot=fs.realpathSync(base),real=fs.realpathSync(file);
    if(!real.startsWith(realRoot+path.sep))return {status:'BLOCKED_EXTERNAL_TARGET',file:relative};
    const st=fs.lstatSync(file);
    if(!st.isFile()||st.isSymbolicLink())return {status:'BLOCKED_NOT_REGULAR_FILE',file:relative};
    if(st.size<=0||st.size>MAX_BYTES)return {status:'BLOCKED_INVALID_SIZE',file:relative,size:st.size};
    const bytes=fs.readFileSync(file);
    if(bytes.length!==st.size)return {status:'BLOCKED_SIZE_RACE',file:relative};
    return {status:'PRESENT_UNREVIEWED',file:relative,size:bytes.length,sha256:crypto.createHash('sha256').update(bytes).digest('hex')};
  }catch(e){
    if(e&&e.code==='ENOENT')return {status:'MISSING',file:relative};
    return {status:'BLOCKED_UNREADABLE',file:relative,error_code:e&&e.code||'UNKNOWN'};
  }
}
function inspectPackage(pkg){
  if(!pkg||typeof pkg.manifest_path!=='string'||!path.isAbsolute(pkg.manifest_path))throw Error('VIDEO18B08: missing absolute manifest path');
  const root=path.dirname(pkg.manifest_path),manifest=fs.realpathSync(pkg.manifest_path);
  if(!path.basename(manifest).endsWith('.toml'))throw Error('VIDEO18B08: invalid manifest');
  const explicit=pkg.license_file;
  // An absolute or parent-traversing Cargo license-file is recorded as unsafe;
  // it is never automatically read outside that package's root.
  const names=explicit?[String(explicit)]:CANDIDATES;
  const files=names.map(n=>fileEvidence(root,n));
  return {name:pkg.name,version:pkg.version,source:pkg.source||'local-source',license_expression:pkg.license||null,declared_license_file:explicit||null,files:files.filter(e=>e.status!=='MISSING'||!!explicit),status:files.some(e=>e.status==='PRESENT_UNREVIEWED')?'TEXT_FOUND_REQUIRES_REVIEW':'TEXT_NOT_VERIFIED'};
}
function buildEvidence(metas,inventory,allow,head){
  if(!/^[0-9a-f]{40}$/.test(head)||!inventory||inventory.release_admitted!==false||inventory.legal_approval!==false||!Array.isArray(inventory.unique_packages))throw Error('VIDEO18B08: invalid inventory or HOLD');
  if(inventory.exact_akula_head!==head)throw Error('VIDEO18B08: inventory HEAD mismatch');
  if(!allow||allow.release_admitted!==false||allow.components?.length!==5||allow.components[2].license!=='AGPL-3.0-only')throw Error('VIDEO18B08: allowlist/HOLD mismatch');
  const all=[
    ...reachablePackages(metas.akula,'aquilum-native'),
    ...reachablePackages(metas.mux,'akula-video-lite-lab'),
    ...reachablePackages(metas.shiva,'cli'),
  ];
  const byKey=new Map();
  for(const pkg of all){
    const key=[pkg.name,pkg.version,pkg.source||'local-source'].join('\u0001');
    if(!byKey.has(key))byKey.set(key,inspectPackage(pkg));
  }
  const evidence=[...byKey.values()].sort((a,b)=>a.name.localeCompare(b.name)||a.version.localeCompare(b.version));
  const wanted=new Set(inventory.unique_packages.map(p=>[p.name,p.version,p.source].join('\u0001')));
  if(wanted.size!==evidence.length||evidence.some(x=>!wanted.has([x.name,x.version,x.source].join('\u0001'))))throw Error('VIDEO18B08: mismatch with full inventory graph');
  return {schema:1,source_head:head,release_admitted:false,legal_approval:false,license_text_review_complete:false,component_count:5,package_count:evidence.length,found_text:evidence.filter(x=>x.status==='TEXT_FOUND_REQUIRES_REVIEW').length,unverified_text:evidence.filter(x=>x.status==='TEXT_NOT_VERIFIED').length,packages:evidence,binary_notices_status:'REVIEW_REQUIRED',notes:'Checks presence, size and SHA256 of license candidates in Cargo package roots. Does not establish terms, notices, rights, or upstream executable bundle contents.'};
}
function run(){
 if(process.argv.length!==9)throw Error('Usage: node script akula.json mux.json shiva.json license-inventory.json allowlist.json sha output.json');
 const [, ,akula,mux,shiva,inventory,allow,head,out]=process.argv;
 const load=p=>JSON.parse(fs.readFileSync(p,'utf8'));
 const result=buildEvidence({akula:load(akula),mux:load(mux),shiva:load(shiva)},load(inventory),load(allow),head);
 fs.mkdirSync(path.dirname(out),{recursive:true});fs.writeFileSync(out,JSON.stringify(result,null,2)+'\n');
 console.log('VIDEO18B08_LICENSE_FILE_EVIDENCE_PASS packages='+result.package_count+' unverified='+result.unverified_text+' release_admitted=0');
}
if(require.main===module)run();
module.exports={fileEvidence,inspectPackage,buildEvidence};
