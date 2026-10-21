'use strict';
const {execFileSync}=require('node:child_process');
const CARGO='aquilum-app/core/Cargo.toml',LIB='aquilum-app/core/src/lib.rs';
function lines(diff){return String(diff).split(/\r?\n/).filter(s=>(s.startsWith('+')&&!s.startsWith('+++'))||(s.startsWith('-')&&!s.startsWith('---')))}
function isolated({files,cargoDiff,libDiff,cargoNew,cargoOld}){
  if(!Array.isArray(files)||!files.some(x=>x.status==='M'&&x.path===CARGO)||!files.some(x=>x.status==='M'&&x.path===LIB))return false;
  if(files.some(x=>!['A','M'].includes(x.status)||(x.status==='M'&&x.path!==CARGO&&x.path!==LIB)))return false;
  const cargo=lines(cargoDiff),lib=lines(libDiff);
  if(lib.length!==2||cargo.length===0||[...cargo,...lib].some(x=>x.startsWith('-')))return false;
  const features=cargo.map(s=>/^\+\s*([a-z][a-z0-9-]+)\s*=\s*\[\s*\]\s*$/.exec(s)).filter(Boolean).map(x=>x[1]);
  if(features.length!==1)return false;
  const feature=features[0],module=feature.replaceAll('-','_');
  if(!feature.endsWith('-probe')&&!feature.endsWith('-lab'))return false;
  const oldSection=/\[features\]([\s\S]*?)(?=\n\[|$)/.exec(cargoOld)?.[1]||'';
  const newSection=/\[features\]([\s\S]*?)(?=\n\[|$)/.exec(cargoNew)?.[1]||'';
  if(oldSection.includes(feature+' = []')||!newSection.includes(feature+' = []'))return false;
  if(newSection.split('\n').some(x=>/^\s*default\s*=/.test(x)&&x.includes(feature)))return false;
  if(lib[0]!=='+#[cfg(feature = "'+feature+'")]'||lib[1]!=='+pub mod '+module+';')return false;
  const added=cargo.filter(x=>!(/^\+\s*(?:#.*)?$/.test(x)));
  const bin=added.includes('+[[bin]]');
  const expected=['+'+feature+' = []'];
  if(bin)expected.push('+[[bin]]','+name = "akula-'+feature+'"','+path = "src/bin/akula_'+module+'.rs"','+required-features = ["'+feature+'"]');
  if(added.length!==expected.length||expected.some(x=>!added.includes(x)))return false;
  const newModule='aquilum-app/core/src/'+module;
  const newBin='aquilum-app/core/src/bin/akula_'+module+'.rs';
  const allowed=new Set([CARGO,LIB,newModule+'.rs','.github/workflows/'+feature+'.yml']);
  if(bin)allowed.add(newBin);
  for(const x of files){
    if(x.status==='M')continue;
    if(!allowed.has(x.path)&&!x.path.startsWith(newModule+'/')&&!x.path.startsWith('docs/integrations/'+feature))return false;
  }
  if(bin&&!files.some(x=>x.status==='A'&&x.path===newBin))return false;
  return files.some(x=>x.status==='A'&&(x.path===newModule+'.rs'||x.path.startsWith(newModule+'/')));
}
function git(args){return execFileSync('git',args,{encoding:'utf8',maxBuffer:8*1024*1024})}
function check(base,head){
  if(!/^[a-f0-9]{40}$/.test(base)||!(head==='HEAD'||/^[a-f0-9]{40}$/.test(head)))throw Error('invalid revision');
  const raw=git(['diff','--name-status','--no-renames',base,head,'--']);
  const files=raw.trim().split('\n').filter(Boolean).map(s=>{const m=/^([ADM])\t(.+)$/.exec(s);return m?{status:m[1],path:m[2]}:{status:'?',path:s}});
  const cargoDiff=git(['diff','--unified=0',base,head,'--',CARGO]);
  const libDiff=git(['diff','--unified=0',base,head,'--',LIB]);
  return isolated({files,cargoDiff,libDiff,cargoNew:git(['show',head+':'+CARGO]),cargoOld:git(['show',base+':'+CARGO])});
}
if(require.main===module){
  try{
    const base=process.argv.find(x=>x.startsWith('--base='))?.slice(7)||'';
    const head=process.argv.find(x=>x.startsWith('--head='))?.slice(7)||'HEAD';
    const skip=check(base,head);
    process.stdout.write('skip='+skip+'\n');
    process.stderr.write('CI_LAB_SCOPE '+(skip?'isolated optional source; diagnostic lab skipped':'full lab required')+'\n');
  }catch(e){
    process.stdout.write('skip=false\n');
    process.stderr.write('CI_LAB_SCOPE failure; run full lab: '+String(e.message).slice(0,170)+'\n');
    process.exitCode=1;
  }
}
module.exports={isolated,check};
