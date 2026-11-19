'use strict';
// VIDEO-TOOLS-01o: NONSHIPPING manual candidate admission against an installed
// higher/lower real mux binary. Not persistent anti-rollback after uninstall.
const assert=require('node:assert/strict');
const crypto=require('node:crypto');
const cp=require('node:child_process');
const fs=require('node:fs');
const path=require('node:path');
const names=['akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe'];
const hex64=/^[0-9a-f]{64}$/;
function hold(msg){throw Error('VIDEO_TOOLS_01O_HOLD: '+msg)}
function version(raw){
  if(typeof raw!=='string'||! /^(0|[1-9]\d{0,9})\.(0|[1-9]\d{0,9})\.(0|[1-9]\d{0,9})$/.test(raw))
    hold('invalid or ambiguous canonical version');
  const v=raw.split('.').map(Number);
  if(v.some(n=>n>0xffffffff))hold('version u32 overflow');
  return v;
}
function newer(a,b){
  a=version(a);b=version(b);
  for(let i=0;i<3;i++){if(a[i]!==b[i])return a[i]>b[i]}
  return false;
}
function sha(bytes){return crypto.createHash('sha256').update(bytes).digest('hex')}
function identity(manifest){
  if(manifest.schema!==1||manifest.product!=='Akula Video Tools'||
     manifest.platform!=='windows-x86_64'||!hex64.test(manifest.cargo_lock_sha256)||
     !/^[a-f0-9]{40}$/.test(manifest.source_commit)||
     !Array.isArray(manifest.binaries)||manifest.binaries.length!==3||
     Object.keys(manifest).sort().join(',')!==
     'binaries,cargo_lock_sha256,platform,product,schema,source_commit,version')
    hold('unexpected signed manifest structure or source identity');
  version(manifest.version);
  for(let i=0;i<3;i++){
    const component=manifest.binaries[i];
    if(Object.keys(component).sort().join(',')!=='name,sha256,size'||
       component.name!==names[i]||!hex64.test(component.sha256)||
       !Number.isSafeInteger(component.size) || component.size<64 ||
       component.size>128*1024*1024)
      hold('invalid signed PE entry');
  }
  return manifest;
}
function signed(root,key,signer,installed){
  const allowed=installed?
    [...names,'manifest.json','manifest.minisig','uninstall.exe']:
    [...names,'manifest.json','manifest.minisig'];
  const actual=fs.readdirSync(root);
  if(actual.length!==allowed.length||
     actual.some(name=>!allowed.includes(name)))hold('non-exact package contents');
  for(const name of actual){
    const stat=fs.lstatSync(path.join(root,name));
    if(!stat.isFile()||stat.isSymbolicLink())hold('aliased or non-file component');
  }
  const manifestPath=path.join(root,'manifest.json');
  const sigPath=path.join(root,'manifest.minisig');
  // Minisign verifies exact bytes, not reconstructed JSON.
  cp.execFileSync(signer,['-Vm',manifestPath,'-x',sigPath,'-p',key,'-Q'],{
    stdio:['ignore','pipe','pipe'],timeout:15000,windowsHide:true});
  const m=identity(JSON.parse(fs.readFileSync(manifestPath,'utf8')));
  for(const b of m.binaries){
    const content=fs.readFileSync(path.join(root,b.name));
    if(content.length!==b.size||sha(content)!==b.sha256)
      hold('real PE bytes not attested by signed manifest: '+b.name);
  }
  return m;
}
function checkFloor(candidate,current){
  // A signed label over identical PE bytes is NOT a real upgrade.
  if(candidate.source_commit!==current.source_commit ||
     candidate.cargo_lock_sha256!==current.cargo_lock_sha256)
    hold('source/lock drift requires a separate acceptance');
  if(!newer(candidate.version,current.version))return 'DENY';
  if(candidate.binaries[0].sha256===current.binaries[0].sha256)
    hold('signed version changed but executable is identical');
  for(let i=1;i<3;i++){
    if(candidate.binaries[i].sha256!==current.binaries[i].sha256)
      hold('upstream components changed without independent review');
  }
  return 'ALLOW';
}
function inside(p,root){
  const a=fs.realpathSync(p),b=fs.realpathSync(root);
  const rel=path.relative(b,a);
  return rel!=='' && rel!=='..' && !rel.startsWith('..'+path.sep) &&
    !path.isAbsolute(rel);
}
function cli(){
  if(process.argv.length!==7)hold('requires candidate, installed, public key, signer and expected verdict');
  if(process.env.GITHUB_ACTIONS!=='true' || process.env.RUNNER_OS!=='Windows' ||
     process.env.RUNNER_ENVIRONMENT!=='github-hosted' ||
     process.env.GITHUB_EVENT_NAME!=='pull_request' ||
     !/^[a-f0-9]{40}$/.test(process.env.EXPECTED_HEAD_SHA))
    hold('exact-head hosted Windows pull-request only');
  const repo=fs.realpathSync(process.env.GITHUB_WORKSPACE||'');
  const head=cp.execFileSync('git',['rev-parse','HEAD'],{
    cwd:repo,encoding:'utf8',stdio:['ignore','pipe','ignore']}).trim();
  if(head!==process.env.EXPECTED_HEAD_SHA)hold('checkout changed');
  const temp=fs.realpathSync(process.env.RUNNER_TEMP||'');
  const [candidate,installed,key,signer,expected]=process.argv.slice(2);
  for(const p of [candidate,key,signer])if(!inside(p,temp))
    hold('caller-selected input escaped runner temp');
  // The wrapper derives the path from the OS-known Program Files root;
  // native Akula independently proves the protected ACL before this test.
  const stat=fs.lstatSync(installed);
  if(!stat.isDirectory()||stat.isSymbolicLink() ||
     path.basename(installed).toLowerCase()!=='akula video tools')
    hold('unexpected installed product root');
  if(!['ALLOW','DENY'].includes(expected))hold('unsupported expected verdict');
  const next=signed(candidate,key,signer,false);
  const current=signed(installed,key,signer,true);
  const verdict=checkFloor(next,current);
  if(verdict!==expected)hold('signed version floor policy differs from expected verdict');
  console.log('VIDEO_TOOLS_01O_SIGNED_FLOOR_'+verdict+
    ' old='+current.version+' candidate='+next.version+
    ' distinct_real_mux='+(current.binaries[0].sha256!==next.binaries[0].sha256?1:0)+
    ' installed_bytes_preserved=1 release=0');
}
module.exports={version,newer,identity,checkFloor};
if(require.main===module){
  try{cli()}catch(e){console.error(String(e.message||e));process.exit(3)}
}
