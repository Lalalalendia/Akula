'use strict';
const {execFileSync} = require('node:child_process');
const fs = require('node:fs');
const prefix = ['aquilum-app/core/src/', 'aquilum-app/native/src/'];
const manifests = ['aquilum-app/core/Cargo.toml','aquilum-app/native/Cargo.toml'];
function own(path) {
  if (!path.endsWith('.rs') || !prefix.some(p=>path.startsWith(p))) return false;
  if (/(^|\/)(vendor|generated|fixtures|tests|test|probes)\//.test(path)) return false;
  return !/(^|\/)(test_.*|.*_tests?\.rs|tests?\.rs|generated\.rs|.*_generated\.rs)$/.test(path);
}
function metrics(text) {
  const lines = text.replace(/\r\n/g,'\n').split('\n');
  if (lines.at(-1)==='') lines.pop();
  const src = lines.map(x=>x.replace(/\/\/.*$/,'')).join('\n');
  const modules = [...new Set([...src.matchAll(/\bcrate::([A-Za-z_]\w*)\b/g)].map(x=>x[1]))];
  return {lines:lines.length, fanout:modules.length,
    exports:(src.match(/^\s*pub(?:\([^\n)]*\))?\s+use\b/gm)||[]).length,
    children:(src.match(/^\s*(?:pub(?:\([^\n)]*\))?\s+)?mod\s+\w+\s*[;{]/gm)||[]).length};
}
function dependencies(text) {
  let inDeps=false;
  const result=new Set();
  for(const line of text.split(/\r?\n/)) {
    const section=line.match(/^\s*\[([^\]]+)\]\s*(?:#.*)?$/);
    if(section) { inDeps=/(?:^|\.)dependencies$/.test(section[1]); continue; }
    const item=inDeps && line.match(/^\s*([A-Za-z_][\w-]*)\s*=/);
    if(item) result.add(item[1]);
  }
  return result;
}
function git(args) {
  return execFileSync('git', args, {encoding:'utf8',maxBuffer:32*1024*1024,stdio:['ignore','pipe','pipe']});
}
function show(ref,path) {
  try {return git(['show',ref+':'+path]);}
  catch(e) {if(e.status===128) return null;throw e;}
}
function compare(path,before,after) {
  const old=before===null?null:metrics(before);
  const now=after===null?null:metrics(after);
  const notes=[];
  if(now) {
    if(old===null && now.lines>600) notes.push('New production module >600 lines');
    if(old && now.lines>800 && now.lines-old.lines>=150) notes.push('Large module grew >=150 lines');
    if(now.fanout>8 && now.fanout-(old?.fanout||0)>=2) notes.push('Outbound root-module breadth increased by >=2 above 8');
    if(now.exports>12 && now.exports-(old?.exports||0)>=3) notes.push('Public reexports increased by >=3 above 12');
    if(now.children>20 && now.children-(old?.children||0)>=3) notes.push('Child modules increased by >=3 above 20');
  }
  return {path,old,now,notes};
}
function assess(rows,newDeps) {
  const netLines=rows.reduce((n,r)=>n+(r.now?.lines||0)-(r.old?.lines||0),0);
  const added=rows.filter(r=>!r.old&&r.now).length;
  const notes=rows.flatMap(r=>r.notes.map(note=>r.path+': '+note));
  if(netLines>500) notes.push('PR: production Rust grows by >500 net lines');
  if(added>10) notes.push('PR: >10 new production modules');
  if(newDeps.length>2) notes.push('Cargo: >2 new direct dependencies');
  return {netLines,added,notes,newDeps,rows};
}
function report(result) {
  const tick='\x60';
  const lines=['## Architecture budget (advisory only)','',
    'Changed owned Rust files: '+result.rows.length+'; net production lines: '+result.netLines+
    '; new modules: '+result.added+'; direct dependency additions: '+result.newDeps.length+'.','',
    '| File | Lines old -> new | Root-module breadth old -> new | Public reexports |',
    '| --- | ---: | ---: | ---: |'];
  for(const r of result.rows.filter(r=>r.now).slice(0,25)) {
    const path=r.path.replace(/[|\r\n]/g,' ');
    lines.push('| '+tick+path+tick+' | '+(r.old?.lines||0)+' -> '+r.now.lines+
      ' | '+(r.old?.fanout||0)+' -> '+r.now.fanout+
      ' | '+(r.old?.exports||0)+' -> '+r.now.exports+' |');
  }
  lines.push('','### Review suggestions (non-blocking)');
  if(!result.notes.length) lines.push('No budget growth warnings.');
  else for(const note of result.notes.slice(0,30)) lines.push('- '+note.replace(/[\r\n]/g,' '));
  lines.push('','Only own production code is counted. Tests/vendor/generated code is excluded. Root-module breadth is a heuristic from crate:: references, not a compiler dependency or call graph; no change fails CI due to its size.','');
  return lines.join('\n');
}
function run(base,head,summary) {
  const changed=git(['diff','--name-only','-z','--find-renames',base,head,'--',
    ...prefix,...manifests]).split('\0').filter(Boolean);
  const rows=changed.filter(own).map(path=>compare(path,show(base,path),show(head,path)));
  const newDeps=[];
  for(const path of changed.filter(x=>manifests.includes(x))) {
    const old=dependencies(show(base,path)||'');
    for(const dep of dependencies(show(head,path)||'')) if(!old.has(dep)) newDeps.push(path+': '+dep);
  }
  const result=assess(rows,newDeps);
  const markdown=report(result);
  process.stdout.write(markdown);
  if(summary) fs.appendFileSync(summary,markdown+'\n');
  return result;
}
if(require.main===module) {
  const args=Object.fromEntries(process.argv.slice(2).map(s=>{
    const i=s.indexOf('=');
    if(!s.startsWith('--')||i<3) throw Error('Use --base=... --head=... [--summary=...]');
    return [s.slice(2,i),s.slice(i+1)];
  }));
  if(!args.base||!args.head) throw Error('Missing --base or --head');
  run(args.base,args.head,args.summary);
}
module.exports={own,metrics,dependencies,compare,assess,report,run};
