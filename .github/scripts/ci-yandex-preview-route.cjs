'use strict';
const {execFileSync}=require('node:child_process');
const fs=require('node:fs');
const path=require('node:path');
// Only two explicitly reviewed *non-Yandex* edits can avoid the costly
// optional Windows preview. Every unknown change keeps the full gate.
const BOOT='aquilum-app/native/src/app/init/bootstrap.rs';
const MOD='aquilum-app/native/src/native_modules.rs';
const ROOT='aquilum-app/';
const OWN=[
  'aquilum-app/native/src/yandex_credentials.rs',
  'aquilum-app/native/src/yandex_probe_session.rs',
  'aquilum-app/native/src/yandex_probe_worker.rs',
  'aquilum-app/native/src/ui/settings/schema/sync/yandex/controls.rs',
  'scripts/test-yandex-credui-cancel.ps1'
];
const ALSO_PROTECTED=new Set([
  'aquilum-app/native/src/main.rs',
  'aquilum-app/native/src/app/mod.rs',
  'aquilum-app/native/src/ui/settings/schema/mod.rs',
  'aquilum-app/native/src/ui/settings/cloud.rs',
  'aquilum-app/native/src/app/settings/cloud_action.rs',
  'aquilum-app/native/src/app/settings/press.rs',
  'aquilum-app/native/src/app/settings/section.rs',
  'aquilum-app/native/src/app/settings/context/build.rs',
  'aquilum-app/native/src/app/lifecycle/tick.rs',
  'aquilum-app/native/src/ui/settings/schema/types/actions.rs',
  'aquilum-app/native/src/ui/settings/schema/types/context/defaults.rs'
]);
// Only PDF-owned leaf code and fixtures may accompany the two exact shared seams.
const PDF_LEAF=/^aquilum-app\/native\/src\/(?:pdf(?:_worker)?\.rs|ui\/pdf\.rs|app\/pdf_view\.rs)$/;
const PDF_HELPER=/^scripts\/(?:pdf-preview\/[a-z0-9-]+\.(?:py|ps1)|install-pdfium-pinned\.ps1|windows-pdf-os-pointer\.ps1)$/;
const MAGIC=/\b(?:yandex|s3|credential|cloud|storage|probe)\b/i;
const safeDelta={
  [BOOT]:['-            tree,','+            tree, pdf: pdf_view::State::new(),'],
  [MOD]:['-mod pdf;','+mod pdf; mod pdf_worker;']
};
function changedLines(diff){
 return String(diff).split(/\r?\n/).filter(s=>(s.startsWith('+')&&!s.startsWith('+++'))||(s.startsWith('-')&&!s.startsWith('---')));
}
function decision(changed,patches){
 if(!Array.isArray(changed)||changed.length===0)return {heavy:true,why:'empty/unknown'};
 if(changed.some(p=>typeof p!=='string'||p.includes('..')))return {heavy:true,why:'invalid path'};
 if(changed.some(p=>p.endsWith('/Cargo.toml')||p.endsWith('/Cargo.lock')||p==='aquilum-app/Cargo.lock'||p==='aquilum-app/Cargo.toml'))return {heavy:true,why:'manifest'};
 if(changed.some(p=>OWN.includes(p)||p.includes('/yandex/')||p.includes('yandex_')||ALSO_PROTECTED.has(p)))return {heavy:true,why:'Yandex/adjacent'};
 if(changed.some(p=>p==='.github/workflows/yandex-s3-native-preview.yml'||p==='.github/scripts/ci-yandex-preview-route.cjs'||p==='.github/scripts/ci-yandex-preview-route.test.cjs'))return {heavy:true,why:'self-policy'};
 let known=0;
 for(const p of changed){
   const lines=changedLines(patches[p]||'');
   if(!lines.length)return {heavy:true,why:'missing diff '+p};
   if(Object.hasOwn(safeDelta,p)){
      if(JSON.stringify(lines)!==JSON.stringify(safeDelta[p]))return {heavy:true,why:'unknown shared edit '+p};
      known++;
   }else {
     if(!PDF_LEAF.test(p)&&!PDF_HELPER.test(p))return {heavy:true,why:'non-PDF change '+p};
     if(lines.some(s=>MAGIC.test(s)))return {heavy:true,why:'sensitive outside owner '+p};
   }
 }
 return {heavy:known===0,why:known===0?'no reviewed shared seam':'reviewed PDF-only shared seam'};
}
function git(...args){return execFileSync('git',args,{encoding:'utf8',maxBuffer:8*1024*1024});}
function compute(base,head){
 if(!/^[0-9a-f]{40}$/.test(base)||!(head==='HEAD'||/^[0-9a-f]{40}$/.test(head)))throw Error('bad SHA');
 const changed=git('diff','--name-only','-z','--no-renames',base,head,'--').split('\0').filter(Boolean);
 const patches=Object.fromEntries(changed.map(p=>[p,git('diff','--unified=0','--no-ext-diff','--no-renames',base,head,'--',p)]));
 return decision(changed,patches);
}
if(require.main===module){
 try{
  const args=Object.fromEntries(process.argv.slice(2).map(s=>{const m=/^--(base|head)=(.*)$/.exec(s);if(!m)throw Error('bad arg');return [m[1],m[2]];}));
  const result=compute(args.base,args.head||'HEAD');
  process.stdout.write('heavy='+result.heavy+'\n');
  console.error('YANDEX_PREVIEW_ROUTE '+JSON.stringify(result));
 }catch(e){
  // Fail closed if git, diff, or parser is unavailable.
  process.stdout.write('heavy=true\n');
  console.error('YANDEX_PREVIEW_ROUTE_FAIL_CLOSED '+e.message);
 }
}
module.exports={decision,changedLines,safeDelta,OWN};
